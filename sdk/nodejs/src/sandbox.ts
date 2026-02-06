import type {
  BatchFileWriteResponse,
  ExecOptions,
  RunOutput,
  SandboxInfo,
} from "./types.js";

type ExecFn = (
  name: string,
  command: string[],
  opts?: ExecOptions,
) => Promise<RunOutput>;
type RemoveFn = (name: string) => Promise<void>;
type GetFn = (name: string) => Promise<SandboxInfo>;
type WriteFilesFn = (
  name: string,
  files: Record<string, string>,
) => Promise<BatchFileWriteResponse>;

/**
 * A sandbox session that auto-removes the sandbox on dispose.
 *
 * Supports both explicit cleanup via remove() and automatic cleanup via
 * Symbol.asyncDispose (TS 5.2+ `await using`).
 *
 * @example
 * ```ts
 * await using sb = await client.sandbox("test");
 * await sb.exec(["echo", "hello"]);
 * // sandbox auto-removed when scope exits
 * ```
 */
export class SandboxSession implements AsyncDisposable {
  readonly name: string;
  private _removed = false;
  private readonly _execInSandbox: ExecFn;
  private readonly _removeSandbox: RemoveFn;
  private readonly _getSandbox: GetFn;
  private readonly _writeFiles: WriteFilesFn;

  /** @internal */
  constructor(
    name: string,
    execInSandbox: ExecFn,
    removeSandbox: RemoveFn,
    getSandbox: GetFn,
    writeFiles: WriteFilesFn,
  ) {
    this.name = name;
    this._execInSandbox = execInSandbox;
    this._removeSandbox = removeSandbox;
    this._getSandbox = getSandbox;
    this._writeFiles = writeFiles;
  }

  /** Run a command in this sandbox. */
  async run(command: string[], opts?: ExecOptions): Promise<RunOutput> {
    return this._execInSandbox(this.name, command, opts);
  }

  /** Get sandbox info. */
  async info(): Promise<SandboxInfo> {
    return this._getSandbox(this.name);
  }

  /** Write multiple files in one request. */
  async writeFiles(
    files: Record<string, string>,
  ): Promise<BatchFileWriteResponse> {
    return this._writeFiles(this.name, files);
  }

  /** Remove the sandbox. Idempotent. */
  async remove(): Promise<void> {
    if (this._removed) return;
    this._removed = true;
    await this._removeSandbox(this.name);
  }

  /** Auto-cleanup for `await using`. */
  async [Symbol.asyncDispose](): Promise<void> {
    await this.remove();
  }
}
