import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { RefreshCw, ChevronRight, ChevronDown, Terminal } from "lucide-react";
import { api } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type { SessionSummary, SessionEntry } from "@/lib/types";

function SessionRow({ entry }: { entry: SessionEntry }) {
  const [expanded, setExpanded] = useState(false);

  return (
    <>
      <TableRow
        className="cursor-pointer"
        onClick={() => setExpanded(!expanded)}
      >
        <TableCell className="w-8 px-2">
          {expanded ? (
            <ChevronDown className="h-3.5 w-3.5 text-muted-foreground" />
          ) : (
            <ChevronRight className="h-3.5 w-3.5 text-muted-foreground" />
          )}
        </TableCell>
        <TableCell className="font-mono text-xs max-w-[400px] truncate">
          $ {entry.command.join(" ")}
        </TableCell>
        <TableCell>
          <Badge variant={entry.exit_code === 0 ? "success" : "destructive"}>
            exit {entry.exit_code}
          </Badge>
        </TableCell>
        <TableCell className="font-mono text-xs">
          {entry.duration_ms}ms
        </TableCell>
        <TableCell className="text-xs text-muted-foreground whitespace-nowrap">
          {new Date(entry.timestamp).toLocaleTimeString()}
        </TableCell>
      </TableRow>
      {expanded && (
        <TableRow>
          <TableCell colSpan={5} className="bg-neutral-950 p-0">
            <pre className="overflow-auto p-4 text-xs font-mono text-neutral-200 leading-relaxed max-h-[300px]">
              {entry.output || <span className="text-neutral-500">No output.</span>}
            </pre>
          </TableCell>
        </TableRow>
      )}
    </>
  );
}

function SandboxSessionPanel({ summary }: { summary: SessionSummary }) {
  const [expanded, setExpanded] = useState(false);

  const { data: session } = useQuery({
    queryKey: ["session", summary.sandbox],
    queryFn: () => api.getSandboxSession(summary.sandbox),
    enabled: expanded,
  });

  return (
    <div className="rounded-md border">
      <button
        className="flex w-full items-center justify-between p-4 text-left hover:bg-muted/50"
        onClick={() => setExpanded(!expanded)}
      >
        <div className="flex items-center gap-3">
          <Terminal className="h-4 w-4 text-muted-foreground" />
          <span className="font-medium">{summary.sandbox}</span>
          <span className="text-xs text-muted-foreground">
            {summary.entry_count} command{summary.entry_count === 1 ? "" : "s"}
          </span>
        </div>
        {expanded ? (
          <ChevronDown className="h-4 w-4 text-muted-foreground" />
        ) : (
          <ChevronRight className="h-4 w-4 text-muted-foreground" />
        )}
      </button>
      {expanded && session && (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead className="w-8 px-2" />
              <TableHead>Command</TableHead>
              <TableHead>Status</TableHead>
              <TableHead>Duration</TableHead>
              <TableHead>Time</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {session.entries.map((entry, idx) => (
              <SessionRow key={idx} entry={entry} />
            ))}
          </TableBody>
        </Table>
      )}
      {expanded && !session && (
        <div className="px-4 pb-4">
          <Skeleton className="h-16 w-full" />
        </div>
      )}
    </div>
  );
}

export function Sessions() {
  const {
    data: sessions,
    isLoading,
    error,
    refetch,
  } = useQuery({
    queryKey: ["sessions"],
    queryFn: () => api.listSessions(),
    refetchInterval: 5000,
  });

  const sorted = useMemo(
    () => sessions ? [...sessions].sort((a, b) => a.sandbox.localeCompare(b.sandbox)) : [],
    [sessions]
  );

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
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Sessions</h1>
          <p className="text-muted-foreground">
            Recorded command history for each sandbox
          </p>
        </div>
        <Button variant="outline" size="sm" onClick={() => refetch()}>
          <RefreshCw className="mr-2 h-4 w-4" />
          Refresh
        </Button>
      </div>

      {error ? (
        <Card>
          <CardContent className="pt-6">
            <p className="text-sm text-destructive">
              Failed to load sessions: {error instanceof Error ? error.message : String(error)}
            </p>
          </CardContent>
        </Card>
      ) : sorted.length === 0 ? (
        <Card>
          <CardContent className="flex flex-col items-center justify-center py-12">
            <Terminal className="h-12 w-12 text-muted-foreground/30 mb-4" />
            <p className="text-muted-foreground">
              No session recordings yet. Execute commands in a sandbox to see them here.
            </p>
          </CardContent>
        </Card>
      ) : (
        <div className="space-y-3">
          {sorted.map((summary) => (
            <SandboxSessionPanel key={summary.sandbox} summary={summary} />
          ))}
        </div>
      )}
    </div>
  );
}
