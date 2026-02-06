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
  BatchCommand,
  BatchFileWriteResponse,
  BatchRunResponse,
  CreateSandboxOptions,
  DetachedCommand,
  DetachedLogsResponse,
  ExecOptions,
  ExtendTtlOptions,
  ExtendTtlResponse,
  FileReadResponse,
  FileWriteOptions,
  RunOptions,
  RunOutput,
  SandboxInfo,
  SnapshotMeta,
  StreamEvent,
  TakeSnapshotOptions,
} from "./types.js";

const SDK_VERSION = "0.3.0";

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
      vcpus: opts?.vcpus,
      memory_mb: opts?.memory_mb,
      profile: opts?.profile,
      source_url: opts?.source_url,
      source_ref: opts?.source_ref,
      volumes: opts?.volumes,
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
  async execInSandbox(
    name: string,
    command: string[],
    opts?: ExecOptions,
  ): Promise<RunOutput> {
    return this.request<RunOutput>(
      "POST",
      `/sandboxes/${encodeURIComponent(name)}/exec`,
      {
        command,
        env: opts?.env,
        workdir: opts?.workdir,
        sudo: opts?.sudo,
      },
    );
  }

  /** Read a file from a sandbox. */
  async readFile(name: string, path: string): Promise<FileReadResponse> {
    return this.request<FileReadResponse>(
      "GET",
      `/sandboxes/${encodeURIComponent(name)}/files/${path}`,
    );
  }

  /** Write a file to a sandbox. */
  async writeFile(
    name: string,
    path: string,
    content: string,
    opts?: FileWriteOptions,
  ): Promise<string> {
    return this.request<string>(
      "PUT",
      `/sandboxes/${encodeURIComponent(name)}/files/${path}`,
      { content, encoding: opts?.encoding ?? "utf8" },
    );
  }

  /** Delete a file from a sandbox. */
  async deleteFile(name: string, path: string): Promise<string> {
    return this.request<string>(
      "DELETE",
      `/sandboxes/${encodeURIComponent(name)}/files/${path}`,
    );
  }

  /** Get audit log entries for a sandbox. */
  async getSandboxLogs(name: string): Promise<Record<string, unknown>[]> {
    return this.request<Record<string, unknown>[]>(
      "GET",
      `/sandboxes/${encodeURIComponent(name)}/logs`,
    );
  }

  /** Write multiple files to a sandbox in one request. */
  async writeFiles(
    name: string,
    files: Record<string, string>,
  ): Promise<BatchFileWriteResponse> {
    return this.request<BatchFileWriteResponse>(
      "POST",
      `/sandboxes/${encodeURIComponent(name)}/files`,
      { files },
    );
  }

  /** Run multiple commands in parallel. */
  async batchRun(commands: BatchCommand[]): Promise<BatchRunResponse> {
    return this.request<BatchRunResponse>("POST", "/batch/run", { commands });
  }

  /** Start a detached (background) command in a sandbox. */
  async execDetached(
    name: string,
    command: string[],
    opts?: ExecOptions,
  ): Promise<DetachedCommand> {
    return this.request<DetachedCommand>(
      "POST",
      `/sandboxes/${encodeURIComponent(name)}/exec/detach`,
      {
        command,
        env: opts?.env,
        workdir: opts?.workdir,
        sudo: opts?.sudo,
      },
    );
  }

  /** Get the status of a detached command. */
  async detachedStatus(
    name: string,
    cmdId: string,
  ): Promise<DetachedCommand> {
    return this.request<DetachedCommand>(
      "GET",
      `/sandboxes/${encodeURIComponent(name)}/exec/detached/${encodeURIComponent(cmdId)}`,
    );
  }

  /** Get logs from a detached command. */
  async detachedLogs(
    name: string,
    cmdId: string,
    stream?: "stdout" | "stderr",
  ): Promise<DetachedLogsResponse> {
    const query = stream === "stderr" ? "?stream=stderr" : "";
    return this.request<DetachedLogsResponse>(
      "GET",
      `/sandboxes/${encodeURIComponent(name)}/exec/detached/${encodeURIComponent(cmdId)}/logs${query}`,
    );
  }

  /** Kill a detached command. */
  async detachedKill(name: string, cmdId: string): Promise<string> {
    return this.request<string>(
      "DELETE",
      `/sandboxes/${encodeURIComponent(name)}/exec/detached/${encodeURIComponent(cmdId)}`,
    );
  }

  /** List detached commands in a sandbox. */
  async detachedList(name: string): Promise<DetachedCommand[]> {
    return this.request<DetachedCommand[]>(
      "GET",
      `/sandboxes/${encodeURIComponent(name)}/exec/detached`,
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
      (n, cmd, o) => this.execInSandbox(n, cmd, o),
      (n) => this.removeSandbox(n),
      (n) => this.getSandbox(n),
      (n, f) => this.writeFiles(n, f),
    );
  }

  // -- TTL & Snapshot methods --

  /** Extend a sandbox's TTL. Returns the new expiry time. */
  async extendTtl(name: string, opts: ExtendTtlOptions): Promise<ExtendTtlResponse> {
    return this.request<ExtendTtlResponse>(
      "POST",
      `/sandboxes/${encodeURIComponent(name)}/extend`,
      { by: opts.by },
    );
  }

  /** List all snapshots. */
  async listSnapshots(): Promise<SnapshotMeta[]> {
    return this.request<SnapshotMeta[]>("GET", "/snapshots");
  }

  /** Take a snapshot of a sandbox. */
  async takeSnapshot(opts: TakeSnapshotOptions): Promise<SnapshotMeta> {
    return this.request<SnapshotMeta>("POST", "/snapshots", opts);
  }

  /** Get info about a snapshot. */
  async getSnapshot(name: string): Promise<SnapshotMeta> {
    return this.request<SnapshotMeta>(
      "GET",
      `/snapshots/${encodeURIComponent(name)}`,
    );
  }

  /** Delete a snapshot. */
  async deleteSnapshot(name: string): Promise<void> {
    await this.request<string>(
      "DELETE",
      `/snapshots/${encodeURIComponent(name)}`,
    );
  }

  /** Restore a sandbox from a snapshot. */
  async restoreSnapshot(name: string): Promise<SandboxInfo> {
    return this.request<SandboxInfo>(
      "POST",
      `/snapshots/${encodeURIComponent(name)}/restore`,
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
