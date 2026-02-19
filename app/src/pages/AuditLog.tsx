import { useState, useMemo } from "react";
import { useSearchParams } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { RefreshCw, Search, ChevronRight, ChevronDown } from "lucide-react";
import { api } from "@/lib/api";
import { toast } from "@/components/ui/use-toast";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type { AuditLogEntry } from "@/lib/types";

function eventBadgeVariant(eventType: string | undefined) {
  if (!eventType) return "secondary" as const;
  if (eventType.includes("removed") || eventType.includes("violation"))
    return "destructive" as const;
  if (eventType.includes("created") || eventType.includes("started"))
    return "success" as const;
  if (eventType.includes("stopped") || eventType.includes("disconnected"))
    return "warning" as const;
  return "secondary" as const;
}

/** Extract the sandbox name from an audit entry's details. */
function sandboxName(entry: AuditLogEntry): string {
  const name = entry.name ?? entry.sandbox;
  return typeof name === "string" ? name : "-";
}

function ExpandableRow({ entry }: { entry: AuditLogEntry }) {
  const [expanded, setExpanded] = useState(false);

  // Build the details object excluding known top-level keys
  const details = useMemo(() => {
    const excluded = new Set(["timestamp", "pid", "user", "type"]);
    const obj: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(entry)) {
      if (!excluded.has(k)) obj[k] = v;
    }
    return obj;
  }, [entry]);

  const hasDetails = Object.keys(details).length > 0;

  return (
    <>
      <TableRow
        className={hasDetails ? "cursor-pointer" : ""}
        onClick={() => hasDetails && setExpanded(!expanded)}
      >
        <TableCell className="w-8 px-2">
          {hasDetails &&
            (expanded ? (
              <ChevronDown className="h-3.5 w-3.5 text-muted-foreground" />
            ) : (
              <ChevronRight className="h-3.5 w-3.5 text-muted-foreground" />
            ))}
        </TableCell>
        <TableCell className="font-mono text-xs whitespace-nowrap">
          {entry.timestamp}
        </TableCell>
        <TableCell>
          <Badge variant={eventBadgeVariant(entry.type)}>
            {entry.type ?? "unknown"}
          </Badge>
        </TableCell>
        <TableCell className="text-sm">{entry.user ?? "-"}</TableCell>
        <TableCell className="font-mono text-xs">
          {sandboxName(entry)}
        </TableCell>
        <TableCell className="text-xs text-muted-foreground max-w-[240px] truncate">
          {summarize(details)}
        </TableCell>
      </TableRow>
      {expanded && (
        <TableRow>
          <TableCell colSpan={6} className="bg-muted/30 p-0">
            <pre className="overflow-auto p-4 text-xs font-mono leading-relaxed">
              {JSON.stringify(entry, null, 2)}
            </pre>
          </TableCell>
        </TableRow>
      )}
    </>
  );
}

/** One-line summary of the detail fields. */
function summarize(details: Record<string, unknown>): string {
  const parts: string[] = [];
  for (const [k, v] of Object.entries(details)) {
    if (v == null) continue;
    const s = Array.isArray(v) ? v.join(" ") : String(v);
    parts.push(`${k}=${s}`);
  }
  return parts.join("  ") || "-";
}

export function AuditLog() {
  const [searchParams] = useSearchParams();
  const [last, setLast] = useState<number>(100);
  const [filter, setFilter] = useState(searchParams.get("filter") ?? "");

  const {
    data: entries,
    isLoading,
    refetch,
  } = useQuery({
    queryKey: ["audit-log", last],
    queryFn: async () => {
      try {
        return await api.getAuditLog(last);
      } catch (err: unknown) {
        toast.error(err instanceof Error ? err.message : String(err));
        throw err;
      }
    },
  });

  const filtered = useMemo(() => {
    if (!entries) return [];
    if (!filter.trim()) return entries;
    const q = filter.toLowerCase();
    return entries.filter((e) => {
      const text = JSON.stringify(e).toLowerCase();
      return text.includes(q);
    });
  }, [entries, filter]);

  if (isLoading) {
    return (
      <div className="space-y-6">
        <Skeleton className="h-10 w-48" />
        <Skeleton className="h-[400px] rounded-lg" />
      </div>
    );
  }

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Audit Log</h1>
          <p className="text-muted-foreground">
            Global operation history across all sandboxes
          </p>
        </div>
        <Button variant="outline" size="sm" onClick={() => refetch()}>
          <RefreshCw className="mr-2 h-4 w-4" />
          Refresh
        </Button>
      </div>

      {/* Controls */}
      <div className="flex items-center gap-3">
        <div className="relative flex-1 max-w-sm">
          <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            placeholder="Filter entries..."
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            className="pl-9"
          />
        </div>
        <Select
          value={String(last)}
          onValueChange={(v) => setLast(Number(v))}
        >
          <SelectTrigger className="w-[140px]">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="50">Last 50</SelectItem>
            <SelectItem value="100">Last 100</SelectItem>
            <SelectItem value="500">Last 500</SelectItem>
          </SelectContent>
        </Select>
        <span className="text-xs text-muted-foreground tabular-nums">
          {filtered.length} / {entries?.length ?? 0} entries
        </span>
      </div>

      {/* Table */}
      <Card>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="w-8 px-2" />
                <TableHead className="whitespace-nowrap">Timestamp</TableHead>
                <TableHead>Event</TableHead>
                <TableHead>User</TableHead>
                <TableHead>Sandbox</TableHead>
                <TableHead>Details</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {filtered.length === 0 ? (
                <TableRow>
                  <TableCell
                    colSpan={6}
                    className="h-24 text-center text-muted-foreground"
                  >
                    {entries?.length === 0
                      ? "No audit entries found"
                      : "No entries match filter"}
                  </TableCell>
                </TableRow>
              ) : (
                filtered.map((entry, idx) => (
                  <ExpandableRow
                    key={`${entry.timestamp}-${idx}`}
                    entry={entry}
                  />
                ))
              )}
            </TableBody>
          </Table>
        </CardContent>
      </Card>
    </div>
  );
}
