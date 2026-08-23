import * as vscode from "vscode";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { AgentKernelClient } from "./client";
import { hostAlias, installManagedSshConfig } from "./sshConfig";
import type { Sandbox } from "./types";

const execFileAsync = promisify(execFile);
const viewId = "agentkernel.sandboxes";
const refreshCommand = "agentkernel.refreshSandboxes";
const connectCommand = "agentkernel.connectToSandbox";

class SandboxItem extends vscode.TreeItem {
  public constructor(public readonly sandbox: Sandbox) {
    super(sandbox.name, vscode.TreeItemCollapsibleState.None);
    this.description = `${sandbox.backend} · ${sandbox.status}`;
    this.tooltip = sandbox.description
      ? `${sandbox.name}\n${sandbox.description}`
      : `${sandbox.name} (${sandbox.backend})`;
    this.contextValue = `agentkernel.sandbox.${sandbox.status}`;
    this.iconPath = new vscode.ThemeIcon(
      sandbox.status === "running" ? "vm-running" : "vm-outline",
    );
    if (sandbox.status === "running") {
      this.command = {
        command: connectCommand,
        title: "Connect to Sandbox",
        arguments: [this],
      };
    }
  }
}

class MessageItem extends vscode.TreeItem {
  public constructor(label: string) {
    super(label, vscode.TreeItemCollapsibleState.None);
    this.contextValue = "agentkernel.message";
  }
}

class SandboxTreeProvider implements vscode.TreeDataProvider<SandboxItem | MessageItem> {
  private readonly changed = new vscode.EventEmitter<void>();
  private sandboxes: Sandbox[] = [];
  private errorMessage?: string;
  private refreshing?: Promise<void>;

  public readonly onDidChangeTreeData = this.changed.event;

  public getTreeItem(item: SandboxItem | MessageItem): vscode.TreeItem {
    return item;
  }

  public getChildren(): Array<SandboxItem | MessageItem> {
    if (this.errorMessage) {
      return [new MessageItem(this.errorMessage)];
    }
    const running = this.sandboxes.filter((sandbox) => sandbox.status === "running");
    return running.length > 0
      ? running.map((sandbox) => new SandboxItem(sandbox))
      : [new MessageItem("No running sandboxes")];
  }

  public runningSandboxes(): Sandbox[] {
    return this.sandboxes.filter((sandbox) => sandbox.status === "running");
  }

  public dispose(): void {
    this.changed.dispose();
  }

  public refresh(): Promise<void> {
    if (this.refreshing) {
      return this.refreshing;
    }
    const refresh = this.load().finally(() => {
      this.refreshing = undefined;
    });
    this.refreshing = refresh;
    return refresh;
  }

  private async load(): Promise<void> {
    this.errorMessage = undefined;
    try {
      const configuration = vscode.workspace.getConfiguration("agentkernel");
      const client = new AgentKernelClient(
        configuration.get<string>("apiUrl", "http://localhost:18888"),
        configuration.get<string>("apiKey", ""),
      );
      this.sandboxes = await client.listSandboxes();
    } catch (error) {
      this.sandboxes = [];
      this.errorMessage = error instanceof Error ? error.message : String(error);
    } finally {
      this.changed.fire();
    }
  }
}

function configuration(): vscode.WorkspaceConfiguration {
  return vscode.workspace.getConfiguration("agentkernel");
}

function cliError(error: unknown): string {
  if (error && typeof error === "object" && "stderr" in error) {
    const stderr = (error as { stderr?: unknown }).stderr;
    if (typeof stderr === "string" && stderr.trim()) {
      return stderr.trim();
    }
  }
  return error instanceof Error ? error.message : String(error);
}

async function generateSshConfig(sandboxName: string): Promise<string> {
  const cliPath = configuration().get<string>("cliPath", "agentkernel").trim();
  if (!cliPath) {
    throw new Error("AgentKernel CLI path is empty. Set agentkernel.cliPath first.");
  }
  try {
    const result = await execFileAsync(cliPath, ["ssh", "config", sandboxName], {
      maxBuffer: 128 * 1024,
      windowsHide: true,
    });
    return result.stdout;
  } catch (error) {
    throw new Error(`Unable to generate SSH config: ${cliError(error)}`);
  }
}

async function openRemoteSsh(host: string): Promise<void> {
  const commands = await vscode.commands.getCommands(true);
  if (commands.includes("opensshremotes.openEmptyWindow")) {
    await vscode.commands.executeCommand("opensshremotes.openEmptyWindow", { host });
    return;
  }
  if (commands.includes("opensshremotes.connectToHost")) {
    await vscode.commands.executeCommand("opensshremotes.connectToHost", host);
    return;
  }
  const action = await vscode.window.showErrorMessage(
    "AgentKernel needs the Microsoft Remote - SSH extension to connect.",
    "Install Remote - SSH",
  );
  if (action === "Install Remote - SSH") {
    await vscode.env.openExternal(
      vscode.Uri.parse(
        "https://marketplace.visualstudio.com/items?itemName=ms-vscode-remote.remote-ssh",
      ),
    );
  }
}

async function connectToSandbox(
  item: SandboxItem | undefined,
  provider: SandboxTreeProvider,
): Promise<void> {
  if (!item) {
    const picked = await vscode.window.showQuickPick(
      provider.runningSandboxes().map((sandbox) => ({
        label: sandbox.name,
        description: `${sandbox.backend} · ${sandbox.status}`,
        sandbox,
      })),
      {
        placeHolder: "Select a running AgentKernel sandbox",
        matchOnDescription: true,
      },
    );
    item = picked ? new SandboxItem(picked.sandbox) : undefined;
  }
  if (!item) {
    return;
  }
  const sandbox = item.sandbox;
  if (sandbox.status !== "running") {
    void vscode.window.showWarningMessage(
      `Sandbox "${sandbox.name}" is not running.`,
    );
    return;
  }

  try {
    const generated = await generateSshConfig(sandbox.name);
    const sshConfigPath = configuration().get<string>(
      "sshConfigPath",
      "~/.ssh/config",
    );
    const resolvedPath = await installManagedSshConfig(
      sshConfigPath,
      sandbox.name,
      generated,
    );
    await openRemoteSsh(hostAlias(sandbox.name));
    void vscode.window.setStatusBarMessage(
      `AgentKernel SSH config updated: ${resolvedPath}`,
      5000,
    );
  } catch (error) {
    void vscode.window.showErrorMessage(
      `Could not connect to "${sandbox.name}": ${
        error instanceof Error ? error.message : String(error)
      }`,
    );
  }
}

export function activate(context: vscode.ExtensionContext): void {
  const provider = new SandboxTreeProvider();
  context.subscriptions.push(
    provider,
    vscode.window.registerTreeDataProvider(viewId, provider),
    vscode.commands.registerCommand(refreshCommand, () => provider.refresh()),
    vscode.commands.registerCommand(
      connectCommand,
      (item: SandboxItem | undefined) => connectToSandbox(item, provider),
    ),
  );

  const refreshInterval = configuration().get<number>("refreshInterval", 30);
  if (refreshInterval > 0) {
    const timer = setInterval(() => void provider.refresh(), refreshInterval * 1000);
    context.subscriptions.push({ dispose: () => clearInterval(timer) });
  }
  void provider.refresh();
}

export function deactivate(): void {
  // No long-lived process or credentials are retained by the extension.
}
