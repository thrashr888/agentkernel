import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  ScrollText,
  RefreshCw,
  ChevronLeft,
  ChevronRight,
} from "lucide-react";
import { api } from "@/lib/api";
import { Button } from "@/components/ui/button";
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

export function PolicyLog() {
  const [page, setPage] = useState(0);
  const [actionFilter, setActionFilter] = useState("all");
  const [decisionFilter, setDecisionFilter] = useState("all");
  const PAGE_SIZE = 20;

  const {
    data: auditEntries,
    isLoading,
    error,
    refetch,
  } = useQuery({
    queryKey: ["policy-audit"],
    queryFn: () => api.getPolicyAudit(500),
    retry: false,
  });

  if (isLoading) {
    return (
      <div className="space-y-6">
        <Skeleton className="h-10 w-48" />
        <Skeleton className="h-[400px] rounded-lg" />
      </div>
    );
  }

  if (error) {
    return (
      <div className="space-y-6">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Policy Log</h1>
          <p className="text-muted-foreground">
            Policy decision audit trail
          </p>
        </div>
        <div className="flex flex-col items-center justify-center py-12 text-center">
          <ScrollText className="mb-4 h-12 w-12 text-muted-foreground/50" />
          <p className="text-sm text-muted-foreground">
            Policy log requires the enterprise feature flag.
          </p>
        </div>
      </div>
    );
  }

  const entries = auditEntries ?? [];
  const actions = [...new Set(entries.map((e) => e.action))].sort();
  const decisions = [...new Set(entries.map((e) => e.decision))].sort();

  const filtered = [...entries]
    .filter((e) => actionFilter === "all" || e.action === actionFilter)
    .filter((e) => decisionFilter === "all" || e.decision === decisionFilter)
    .reverse();

  const start = page * PAGE_SIZE;
  const pageSlice = filtered.slice(start, start + PAGE_SIZE);

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">Policy Log</h1>
          <p className="text-muted-foreground">
            Authorization decision audit trail
          </p>
        </div>
        <Button variant="outline" size="sm" onClick={() => refetch()}>
          <RefreshCw className="mr-2 h-4 w-4" />
          Refresh
        </Button>
      </div>

      {entries.length === 0 ? (
        <div className="flex flex-col items-center justify-center py-12 text-center">
          <ScrollText className="mb-3 h-8 w-8 text-muted-foreground/50" />
          <p className="text-sm text-muted-foreground">
            No policy decisions recorded yet.
          </p>
          <p className="text-xs text-muted-foreground">
            Use the Policy Check tester or make sandbox API calls to generate
            audit entries.
          </p>
        </div>
      ) : (
        <>
          {/* Filters */}
          <div className="flex items-center gap-3">
            <Select
              value={actionFilter}
              onValueChange={(v) => {
                setActionFilter(v);
                setPage(0);
              }}
            >
              <SelectTrigger className="w-[180px]">
                <SelectValue placeholder="All actions" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="all">All actions</SelectItem>
                {actions.map((a) => (
                  <SelectItem key={a} value={a}>
                    {a}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <Select
              value={decisionFilter}
              onValueChange={(v) => {
                setDecisionFilter(v);
                setPage(0);
              }}
            >
              <SelectTrigger className="w-[180px]">
                <SelectValue placeholder="All decisions" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="all">All decisions</SelectItem>
                {decisions.map((d) => (
                  <SelectItem key={d} value={d}>
                    {d}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            {(actionFilter !== "all" || decisionFilter !== "all") && (
              <span className="text-xs text-muted-foreground">
                {filtered.length} of {entries.length}
              </span>
            )}
          </div>

          {/* Table */}
          <div className="rounded-md border">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead className="w-[160px]">Time</TableHead>
                  <TableHead>Principal</TableHead>
                  <TableHead>Action</TableHead>
                  <TableHead>Resource</TableHead>
                  <TableHead className="w-[90px]">Decision</TableHead>
                  <TableHead>Reason</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {pageSlice.map((entry, i) => (
                  <TableRow key={start + i}>
                    <TableCell className="font-mono text-xs text-muted-foreground">
                      {new Date(entry.timestamp).toLocaleString(undefined, {
                        month: "short",
                        day: "numeric",
                        hour: "2-digit",
                        minute: "2-digit",
                        second: "2-digit",
                      })}
                    </TableCell>
                    <TableCell className="font-mono text-xs">
                      {entry.principal}
                    </TableCell>
                    <TableCell>
                      <Badge variant="outline" className="font-mono text-xs">
                        {entry.action}
                      </Badge>
                    </TableCell>
                    <TableCell className="font-mono text-xs">
                      {entry.resource}
                    </TableCell>
                    <TableCell>
                      <Badge
                        variant={
                          entry.decision === "Permit"
                            ? "success"
                            : "destructive"
                        }
                      >
                        {entry.decision}
                      </Badge>
                    </TableCell>
                    <TableCell className="max-w-[200px] truncate text-xs text-muted-foreground">
                      {entry.reason ?? ""}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>

          {/* Pagination */}
          {filtered.length > PAGE_SIZE && (() => {
            const totalPages = Math.ceil(filtered.length / PAGE_SIZE);
            return (
              <div className="flex items-center justify-between">
                <span className="text-sm text-muted-foreground">
                  {page * PAGE_SIZE + 1}&ndash;
                  {Math.min((page + 1) * PAGE_SIZE, filtered.length)} of{" "}
                  {filtered.length}
                </span>
                <div className="flex items-center gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => setPage((p) => p - 1)}
                    disabled={page === 0}
                  >
                    <ChevronLeft className="mr-1 h-4 w-4" />
                    Previous
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => setPage((p) => p + 1)}
                    disabled={page >= totalPages - 1}
                  >
                    Next
                    <ChevronRight className="ml-1 h-4 w-4" />
                  </Button>
                </div>
              </div>
            );
          })()}
        </>
      )}
    </div>
  );
}
