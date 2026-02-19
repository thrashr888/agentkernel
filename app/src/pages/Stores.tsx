import { useState } from "react";
import {
  Trash2,
  AlertTriangle,
  Database,
  RefreshCw,
  Plus,
  Play,
  ArrowLeft,
  Terminal,
  Copy,
  CheckCircle2,
  XCircle,
  Loader2,
} from "lucide-react";
import { useMutation, useQueryClient, useQuery } from "@tanstack/react-query";
import { useStores } from "@/lib/hooks/use-stores";
import { api } from "@/lib/api";
import { toast } from "@/components/ui/use-toast";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { DurableStoreInfo, StoreQueryResult } from "@/lib/types";
import { Link } from "react-router-dom";
import { SandboxPicker } from "@/components/sandbox-picker";

function kindBadge(kind: string) {
  switch (kind) {
    case "sqlite":
      return <Badge variant="default">SQLite</Badge>;
    case "postgres":
      return <Badge variant="secondary">Postgres</Badge>;
    case "mysql":
      return <Badge variant="secondary">MySQL</Badge>;
    case "redis":
      return <Badge variant="warning">Redis</Badge>;
    default:
      return <Badge variant="outline">{kind}</Badge>;
  }
}

// --- SQLite Console ---
function SqlConsole({ store, label, placeholder }: {
  store: DurableStoreInfo;
  label: string;
  placeholder: string;
}) {
  const [sql, setSql] = useState("");
  const [queryResult, setQueryResult] = useState<StoreQueryResult | null>(null);
  const [queryError, setQueryError] = useState<string | null>(null);

  const queryMutation = useMutation({
    mutationFn: () => api.queryStore(store.id, sql),
    onSuccess: (result) => {
      setQueryResult(result);
      setQueryError(null);
    },
    onError: (err) => {
      setQueryError(err instanceof Error ? err.message : String(err));
      setQueryResult(null);
    },
  });

  const executeMutation = useMutation({
    mutationFn: () => api.executeStore(store.id, sql),
    onSuccess: (result) => {
      setQueryResult(null);
      setQueryError(null);
      toast.success(`${result.rows_affected} row(s) affected`);
    },
    onError: (err) => {
      setQueryError(err instanceof Error ? err.message : String(err));
      setQueryResult(null);
    },
  });

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Terminal className="h-4 w-4" />
          {label} Console
        </CardTitle>
        <CardDescription>
          Run SQL queries against this {label} store
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="grid gap-2">
          <Label htmlFor="sql-query">SQL</Label>
          <textarea
            id="sql-query"
            value={sql}
            onChange={(e) => setSql(e.target.value)}
            placeholder={placeholder}
            className="flex min-h-[100px] w-full rounded-md border border-input bg-background px-3 py-2 text-sm font-mono ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
            rows={4}
          />
        </div>
        <div className="flex gap-2">
          <Button
            variant="outline"
            onClick={() => queryMutation.mutate()}
            disabled={!sql.trim() || queryMutation.isPending}
          >
            <Play className="h-4 w-4 mr-1.5" />
            {queryMutation.isPending ? "Running..." : "Query"}
          </Button>
          <Button
            variant="outline"
            onClick={() => executeMutation.mutate()}
            disabled={!sql.trim() || executeMutation.isPending}
          >
            {executeMutation.isPending ? "Executing..." : "Execute"}
          </Button>
        </div>

        {queryError && (
          <div className="rounded-md border border-destructive/50 bg-destructive/10 p-3">
            <p className="text-sm text-destructive font-mono">{queryError}</p>
          </div>
        )}

        {queryResult && (
          <div className="space-y-2">
            <p className="text-sm text-muted-foreground">
              {queryResult.row_count} row(s) returned
            </p>
            {queryResult.row_count > 0 && (
              <div className="overflow-auto rounded-md border">
                <Table>
                  <TableHeader>
                    <TableRow>
                      {queryResult.columns.map((col) => (
                        <TableHead key={col} className="font-mono text-xs">
                          {col}
                        </TableHead>
                      ))}
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {queryResult.rows.map((row, i) => (
                      <TableRow key={i}>
                        {queryResult.columns.map((col) => (
                          <TableCell key={col} className="font-mono text-xs">
                            {String(
                              (row as Record<string, unknown>)[col] ?? "NULL"
                            )}
                          </TableCell>
                        ))}
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              </div>
            )}
          </div>
        )}
      </CardContent>
    </Card>
  );
}

// --- Redis Console ---
function RedisConsole({ store }: { store: DurableStoreInfo }) {
  const [commandInput, setCommandInput] = useState("");
  const [history, setHistory] = useState<{ cmd: string; result: string; error?: boolean }[]>([]);

  const commandMutation = useMutation({
    mutationFn: (command: string[]) => api.commandStore(store.id, command),
    onSuccess: (result, command) => {
      setHistory((prev) => [
        ...prev,
        { cmd: command.join(" "), result: formatRedisResult(result.result) },
      ]);
      setCommandInput("");
    },
    onError: (err, command) => {
      setHistory((prev) => [
        ...prev,
        {
          cmd: command.join(" "),
          result: err instanceof Error ? err.message : String(err),
          error: true,
        },
      ]);
      setCommandInput("");
    },
  });

  function runCommand(parts: string[]) {
    commandMutation.mutate(parts);
  }

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    const parts = commandInput.trim().split(/\s+/);
    if (parts.length === 0 || !parts[0]) return;
    runCommand(parts);
  }

  const quickCommands = [
    { label: "PING", cmd: ["PING"] },
    { label: "INFO", cmd: ["INFO", "server"] },
    { label: "KEYS *", cmd: ["KEYS", "*"] },
    { label: "DBSIZE", cmd: ["DBSIZE"] },
  ];

  const config = store.config as Record<string, unknown> | null;
  const host = (config?.host as string) || "127.0.0.1";
  const port = (config?.port as number) || 6379;

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Terminal className="h-4 w-4" />
          Redis Console
        </CardTitle>
        <CardDescription>
          Connected to {host}:{port}
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="flex flex-wrap gap-1.5">
          {quickCommands.map((qc) => (
            <Button
              key={qc.label}
              variant="secondary"
              size="sm"
              className="font-mono text-xs h-7"
              onClick={() => runCommand(qc.cmd)}
              disabled={commandMutation.isPending}
            >
              {qc.label}
            </Button>
          ))}
        </div>
        {history.length > 0 && (
          <div className="overflow-auto rounded-md bg-muted p-3 max-h-[300px] font-mono text-xs space-y-1">
            {history.map((entry, i) => (
              <div key={i}>
                <div className="text-primary">&gt; {entry.cmd}</div>
                <div className={entry.error ? "text-destructive" : "text-foreground whitespace-pre-wrap"}>
                  {entry.result}
                </div>
              </div>
            ))}
          </div>
        )}
        <form onSubmit={handleSubmit} className="flex gap-2">
          <Input
            value={commandInput}
            onChange={(e) => setCommandInput(e.target.value)}
            placeholder="GET key / SET key value / HGETALL hash / DEL key"
            className="font-mono"
            disabled={commandMutation.isPending}
          />
          <Button type="submit" variant="outline" disabled={!commandInput.trim() || commandMutation.isPending}>
            <Play className="h-4 w-4 mr-1.5" />
            Run
          </Button>
        </form>
      </CardContent>
    </Card>
  );
}

function formatRedisResult(value: unknown): string {
  if (value === null || value === undefined) return "(nil)";
  if (typeof value === "string") return `"${value}"`;
  if (typeof value === "number") return `(integer) ${value}`;
  if (Array.isArray(value)) {
    if (value.length === 0) return "(empty array)";
    return value.map((v, i) => `${i + 1}) ${formatRedisResult(v)}`).join("\n");
  }
  return JSON.stringify(value, null, 2);
}

// --- Postgres / MySQL Console (sandbox-based) ---
const SQL_PLACEHOLDERS: Record<string, string> = {
  sqlite: "SELECT * FROM sqlite_master;\nCREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT);",
  postgres: "SELECT * FROM information_schema.tables WHERE table_schema = 'public';\nCREATE TABLE items (id SERIAL PRIMARY KEY, name TEXT);",
  mysql: "SHOW TABLES;\nCREATE TABLE items (id INT AUTO_INCREMENT PRIMARY KEY, name VARCHAR(255));",
};

const SQL_LABELS: Record<string, string> = {
  sqlite: "SQLite",
  postgres: "PostgreSQL",
  mysql: "MySQL",
};

// --- Connection Info ---
function getConnectionHelp(
  kind: string,
  host: string,
  sandbox?: string,
): { cli: string; uri: string; desc: string } | null {
  switch (kind) {
    case "postgres":
      return {
        cli: `psql -h ${host} -p 5432 -U postgres`,
        uri: `postgresql://postgres@${host}:5432/postgres`,
        desc: `PostgreSQL running in sandbox "${sandbox}". Default user: postgres, default database: postgres.`,
      };
    case "mysql":
      return {
        cli: `mysql -h ${host} -P 3306 -u root`,
        uri: `mysql://root@${host}:3306`,
        desc: `MySQL running in sandbox "${sandbox}". Default user: root (no password).`,
      };
    case "redis":
      return {
        cli: `redis-cli -h ${host} -p 6379`,
        uri: `redis://${host}:6379`,
        desc: `Redis running in sandbox "${sandbox}".`,
      };
    case "sqlite":
      return {
        cli: "sqlite3 <path>",
        uri: "",
        desc: "Embedded SQLite database. Query directly via the console below.",
      };
    default:
      return null;
  }
}

function copySnippet(text: string) {
  navigator.clipboard.writeText(text);
  toast.success("Copied to clipboard");
}

function CopyableSnippet({ label, text }: { label: string; text: string }) {
  return (
    <div className="space-y-1">
      <div className="flex items-center justify-between">
        <span className="text-xs font-medium text-muted-foreground">
          {label}
        </span>
        <Button
          variant="ghost"
          size="sm"
          className="h-6 px-2"
          onClick={() => copySnippet(text)}
        >
          <Copy className="h-3 w-3" />
        </Button>
      </div>
      <pre className="overflow-auto rounded-md bg-muted px-3 py-2 text-xs font-mono whitespace-pre-wrap">
        {text}
      </pre>
    </div>
  );
}

// --- Store Detail ---
function StoreDetail({
  store,
  onBack,
}: {
  store: DurableStoreInfo;
  onBack: () => void;
}) {
  const { data: sandboxInfo, isLoading: sandboxLoading } = useQuery({
    queryKey: ["sandboxes", store.sandbox, "info"],
    queryFn: async () => {
      const sandboxes = await api.listSandboxes();
      return sandboxes.find((s) => s.name === store.sandbox) ?? null;
    },
    enabled: !!store.sandbox,
    refetchInterval: 5000,
  });

  const config = store.config as Record<string, unknown> | null;

  // Use sandbox IP if available, fall back to localhost for port-mapped access
  const host = sandboxInfo?.ip || "localhost";
  const help = getConnectionHelp(store.kind, host, store.sandbox ?? undefined);
  const sandboxRunning =
    sandboxInfo?.status.toLowerCase() === "running";

  return (
    <div className="space-y-6">
      <div className="flex items-center gap-3">
        <Button variant="ghost" size="sm" onClick={onBack}>
          <ArrowLeft className="h-4 w-4 mr-1" />
          Back
        </Button>
        <div>
          <h1 className="text-3xl font-bold tracking-tight">{store.name}</h1>
          <p className="text-muted-foreground flex items-center gap-2">
            {kindBadge(store.kind)}
            <span className="text-sm">
              Created {new Date(store.created_at).toLocaleString()}
            </span>
            {store.sandbox && (
              <Link
                to={`/sandboxes/${store.sandbox}`}
                className="text-sm text-primary hover:underline"
              >
                {store.sandbox}
              </Link>
            )}
          </p>
        </div>
      </div>

      {/* Connection info card */}
      <Card>
        <CardHeader>
          <div className="flex items-start justify-between">
            <div>
              <CardTitle className="flex items-center gap-2">
                <Database className="h-4 w-4" />
                Connection
              </CardTitle>
              {help && <CardDescription>{help.desc}</CardDescription>}
            </div>
            {store.sandbox && (
              <div>
                {sandboxLoading ? (
                  <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
                    <Loader2 className="h-3 w-3 animate-spin" />
                    Checking...
                  </div>
                ) : !sandboxInfo ? (
                  <div className="flex items-center gap-1.5 text-xs text-destructive">
                    <XCircle className="h-3 w-3" />
                    Sandbox not found
                  </div>
                ) : (
                  <div
                    className={`flex items-center gap-1.5 text-xs ${sandboxRunning ? "text-green-600 dark:text-green-400" : "text-destructive"}`}
                  >
                    {sandboxRunning ? (
                      <CheckCircle2 className="h-3 w-3" />
                    ) : (
                      <XCircle className="h-3 w-3" />
                    )}
                    {sandboxRunning ? "Running" : "Stopped"}
                    {sandboxInfo.ip && (
                      <span className="text-muted-foreground font-mono ml-1">
                        ({sandboxInfo.ip})
                      </span>
                    )}
                  </div>
                )}
              </div>
            )}
          </div>
        </CardHeader>
        <CardContent className="space-y-3">
          <dl className="space-y-3 text-sm">
            <div className="flex justify-between">
              <dt className="text-muted-foreground">Store ID</dt>
              <dd className="font-mono text-xs">{store.id}</dd>
            </div>
            <div className="flex justify-between">
              <dt className="text-muted-foreground">Kind</dt>
              <dd>{kindBadge(store.kind)}</dd>
            </div>
            {store.sandbox && (
              <div className="flex justify-between">
                <dt className="text-muted-foreground">Sandbox</dt>
                <dd>
                  <Link
                    to={`/sandboxes/${store.sandbox}`}
                    className="text-primary hover:underline font-mono"
                  >
                    {store.sandbox}
                  </Link>
                </dd>
              </div>
            )}
            {config && Object.keys(config).length > 0 && (
              <div className="flex justify-between">
                <dt className="text-muted-foreground">Config</dt>
                <dd className="font-mono text-xs max-w-[300px] truncate">
                  {JSON.stringify(config)}
                </dd>
              </div>
            )}
          </dl>

          {help && (
            <div className="space-y-2 pt-2">
              <CopyableSnippet label="CLI" text={help.cli} />
              {help.uri && (
                <CopyableSnippet label="Connection URI" text={help.uri} />
              )}
              <CopyableSnippet
                label="API Query"
                text={`curl -X POST http://localhost:18888/stores/${store.id}/query \\\n  -H "Content-Type: application/json" \\\n  -d '{"sql": "SELECT 1"}'`}
              />
            </div>
          )}
        </CardContent>
      </Card>

      {(store.kind === "sqlite" ||
        store.kind === "postgres" ||
        store.kind === "mysql") && (
        <SqlConsole
          store={store}
          label={SQL_LABELS[store.kind] ?? store.kind}
          placeholder={SQL_PLACEHOLDERS[store.kind] ?? "SELECT 1;"}
        />
      )}
      {store.kind === "redis" && <RedisConsole store={store} />}
    </div>
  );
}

// --- Store List ---
export function Stores() {
  const queryClient = useQueryClient();
  const { data: stores, isLoading, error, refetch, isRefetching } = useStores();

  const [dialogOpen, setDialogOpen] = useState(false);
  const [newName, setNewName] = useState("");
  const [newKind, setNewKind] = useState("sqlite");
  const [newSandbox, setNewSandbox] = useState("");

  const [selectedStore, setSelectedStore] = useState<DurableStoreInfo | null>(null);

  const createMutation = useMutation({
    mutationFn: () =>
      api.createStore({
        name: newName.trim(),
        kind: newKind,
        sandbox: newSandbox.trim() || undefined,
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["stores"] });
      setNewName("");
      setNewKind("sqlite");
      setNewSandbox("");
      setDialogOpen(false);
      toast.success("Store created");
    },
    onError: (err) => {
      toast.error(err instanceof Error ? err.message : String(err));
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => api.deleteStore(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["stores"] });
      toast.success("Store deleted");
    },
    onError: (err) => {
      toast.error(err instanceof Error ? err.message : String(err));
    },
  });

  if (selectedStore) {
    return (
      <StoreDetail
        store={selectedStore}
        onBack={() => setSelectedStore(null)}
      />
    );
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Durable Stores</h1>
          <p className="text-muted-foreground">
            Manage persistent data stores for sandboxes
          </p>
        </div>
        <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
          <DialogTrigger asChild>
            <Button>
              <Plus className="h-4 w-4 mr-2" />
              New Store
            </Button>
          </DialogTrigger>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>Create Durable Store</DialogTitle>
              <DialogDescription>
                Create a new persistent data store.
              </DialogDescription>
            </DialogHeader>
            <div className="space-y-3">
              <div className="grid gap-2">
                <Label htmlFor="store-name">Name</Label>
                <Input
                  id="store-name"
                  value={newName}
                  onChange={(e) => setNewName(e.target.value)}
                  placeholder="agent-state"
                />
              </div>
              <div className="grid gap-2">
                <Label htmlFor="store-kind">Kind</Label>
                <Select value={newKind} onValueChange={setNewKind}>
                  <SelectTrigger>
                    <SelectValue placeholder="Select kind" />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="sqlite">SQLite</SelectItem>
                    <SelectItem value="postgres">Postgres</SelectItem>
                    <SelectItem value="mysql">MySQL</SelectItem>
                    <SelectItem value="redis">Redis</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              <div className="grid gap-2">
                <Label>Sandbox (optional)</Label>
                <SandboxPicker
                  value={newSandbox}
                  onChange={setNewSandbox}
                />
              </div>
            </div>
            <DialogFooter>
              <Button
                onClick={() => createMutation.mutate()}
                disabled={!newName.trim() || createMutation.isPending}
              >
                {createMutation.isPending ? "Creating..." : "Create"}
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Stores</CardTitle>
          <CardDescription>
            Durable stores provide persistent data backends
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          {isLoading && (
            <div className="space-y-2">
              <Skeleton className="h-8 w-full" />
              <Skeleton className="h-8 w-full" />
            </div>
          )}

          {error && (
            <div className="rounded-md border border-destructive/50 bg-destructive/10 p-4">
              <div className="flex items-start gap-3">
                <AlertTriangle className="h-5 w-5 text-destructive mt-0.5 shrink-0" />
                <div className="flex-1 space-y-2">
                  <p className="text-sm font-medium text-destructive">
                    Failed to load stores
                  </p>
                  <p className="text-xs text-destructive/80">
                    {error instanceof Error ? error.message : String(error)}
                  </p>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => refetch()}
                    disabled={isRefetching}
                    className="mt-1"
                  >
                    <RefreshCw className={`h-3.5 w-3.5 mr-1.5 ${isRefetching ? "animate-spin" : ""}`} />
                    {isRefetching ? "Retrying..." : "Retry"}
                  </Button>
                </div>
              </div>
            </div>
          )}

          {stores && stores.length > 0 && (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Name</TableHead>
                  <TableHead>Kind</TableHead>
                  <TableHead>Sandbox</TableHead>
                  <TableHead>Created</TableHead>
                  <TableHead className="w-[120px] text-right">Actions</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {stores.map((store) => (
                  <TableRow key={store.id}>
                    <TableCell>
                      <button
                        onClick={() => setSelectedStore(store)}
                        className="font-medium text-primary hover:underline text-left"
                      >
                        {store.name}
                      </button>
                    </TableCell>
                    <TableCell>{kindBadge(store.kind)}</TableCell>
                    <TableCell>
                      {store.sandbox ? (
                        <Link
                          to={`/sandboxes/${store.sandbox}`}
                          className="text-primary hover:underline text-sm"
                        >
                          {store.sandbox}
                        </Link>
                      ) : (
                        <span className="text-muted-foreground text-sm">—</span>
                      )}
                    </TableCell>
                    <TableCell className="text-sm text-muted-foreground">
                      {new Date(store.created_at).toLocaleString()}
                    </TableCell>
                    <TableCell className="text-right">
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => deleteMutation.mutate(store.id)}
                        disabled={deleteMutation.isPending}
                      >
                        <Trash2 className="h-4 w-4" />
                      </Button>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}

          {stores && stores.length === 0 && !isLoading && (
            <div className="flex flex-col items-center gap-2 py-4 text-center">
              <Database className="h-8 w-8 text-muted-foreground/50" />
              <p className="text-sm text-muted-foreground">No stores yet</p>
              <p className="text-xs text-muted-foreground/70">
                Create a durable store using the button above or via the API.
              </p>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
