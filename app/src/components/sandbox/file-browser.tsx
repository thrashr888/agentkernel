import { useState, useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { Folder, File, RefreshCw, ChevronRight, Loader2, X } from "lucide-react";
import { api } from "@/lib/api";
import { Button } from "@/components/ui/button";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { toast } from "@/components/ui/use-toast";

interface FileEntry {
  permissions: string;
  links: string;
  owner: string;
  group: string;
  size: string;
  date: string;
  name: string;
  isDirectory: boolean;
  isSymlink: boolean;
}

function parseLsOutput(output: string): FileEntry[] {
  const lines = output.split("\n").filter((l) => l.trim().length > 0);
  const entries: FileEntry[] = [];

  for (const line of lines) {
    // Skip the "total NNN" line
    if (line.startsWith("total ")) continue;

    // ls -la output: permissions links owner group size month day time/year name
    const parts = line.split(/\s+/);
    if (parts.length < 9) continue;

    const permissions = parts[0];
    const links = parts[1];
    const owner = parts[2];
    const group = parts[3];
    const size = parts[4];
    const date = `${parts[5]} ${parts[6]} ${parts[7]}`;
    // Name may contain spaces, so join the rest
    const name = parts.slice(8).join(" ");

    // Skip . and .. entries here (we handle ".." separately as a back button)
    if (name === "." || name === "..") continue;

    const isDirectory = permissions.startsWith("d");
    const isSymlink = permissions.startsWith("l");

    entries.push({
      permissions,
      links,
      owner,
      group,
      size,
      date,
      name: isSymlink && name.includes(" -> ") ? name.split(" -> ")[0] : name,
      isDirectory,
      isSymlink,
    });
  }

  // Sort: directories first, then files, alphabetical within each group
  entries.sort((a, b) => {
    if (a.isDirectory && !b.isDirectory) return -1;
    if (!a.isDirectory && b.isDirectory) return 1;
    return a.name.localeCompare(b.name);
  });

  return entries;
}

function formatFileSize(sizeStr: string): string {
  const size = parseInt(sizeStr, 10);
  if (isNaN(size)) return sizeStr;
  if (size < 1024) return `${size} B`;
  if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KB`;
  if (size < 1024 * 1024 * 1024) return `${(size / (1024 * 1024)).toFixed(1)} MB`;
  return `${(size / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

interface FileBrowserProps {
  sandboxName: string;
}

export function FileBrowser({ sandboxName }: FileBrowserProps) {
  const [currentPath, setCurrentPath] = useState("/");
  const [selectedFile, setSelectedFile] = useState<string | null>(null);

  const {
    data: listData,
    isLoading: isListLoading,
    error: listError,
    refetch: refetchList,
  } = useQuery({
    queryKey: ["files-list", sandboxName, currentPath],
    queryFn: () => api.listFiles(sandboxName, currentPath),
    enabled: !!sandboxName,
  });

  const {
    data: fileContent,
    isLoading: isFileLoading,
    error: fileError,
  } = useQuery({
    queryKey: ["file-read", sandboxName, selectedFile],
    queryFn: () => api.readFile(sandboxName, selectedFile!),
    enabled: !!sandboxName && !!selectedFile,
  });

  // Show file errors as toasts
  if (fileError) {
    toast.error(fileError instanceof Error ? fileError.message : String(fileError));
  }

  const entries = useMemo(() => {
    if (!listData?.output) return [];
    return parseLsOutput(listData.output);
  }, [listData]);

  const breadcrumbs = useMemo(() => {
    const parts = currentPath.split("/").filter(Boolean);
    const crumbs: { label: string; path: string }[] = [{ label: "/", path: "/" }];
    let accumulated = "";
    for (const part of parts) {
      accumulated += `/${part}`;
      crumbs.push({ label: part, path: accumulated });
    }
    return crumbs;
  }, [currentPath]);

  function navigateTo(path: string) {
    setSelectedFile(null);
    setCurrentPath(path);
  }

  function goUp() {
    if (currentPath === "/") return;
    const parts = currentPath.split("/").filter(Boolean);
    parts.pop();
    navigateTo(parts.length === 0 ? "/" : `/${parts.join("/")}`);
  }

  function handleEntryClick(entry: FileEntry) {
    if (entry.isDirectory) {
      const newPath =
        currentPath === "/"
          ? `/${entry.name}`
          : `${currentPath}/${entry.name}`;
      navigateTo(newPath);
    } else {
      const fullPath =
        currentPath === "/"
          ? `/${entry.name}`
          : `${currentPath}/${entry.name}`;
      setSelectedFile(fullPath);
    }
  }

  return (
    <div className="space-y-4">
      {/* Breadcrumb + Refresh */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-1 text-sm font-mono">
          {breadcrumbs.map((crumb, i) => (
            <span key={crumb.path} className="flex items-center gap-1">
              {i > 0 && (
                <ChevronRight className="h-3 w-3 text-muted-foreground" />
              )}
              <button
                type="button"
                className="hover:underline text-foreground"
                onClick={() => navigateTo(crumb.path)}
              >
                {crumb.label}
              </button>
            </span>
          ))}
        </div>
        <Button
          variant="outline"
          size="sm"
          onClick={() => refetchList()}
          disabled={isListLoading}
        >
          <RefreshCw
            className={`mr-2 h-3 w-3 ${isListLoading ? "animate-spin" : ""}`}
          />
          Refresh
        </Button>
      </div>

      {/* Error state */}
      {listError && (
        <div className="rounded-md border border-destructive/50 bg-destructive/10 p-4 text-sm text-destructive">
          Failed to list files:{" "}
          {listError instanceof Error ? listError.message : String(listError)}
        </div>
      )}

      {/* Loading state */}
      {isListLoading && (
        <div className="flex items-center justify-center py-12">
          <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
          <span className="ml-2 text-sm text-muted-foreground">
            Loading files...
          </span>
        </div>
      )}

      {/* File listing table */}
      {!isListLoading && !listError && (
        <div className="rounded-md border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="w-[50%]">Name</TableHead>
                <TableHead className="w-[20%]">Size</TableHead>
                <TableHead className="w-[30%]">Permissions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {/* Back / parent directory */}
              {currentPath !== "/" && (
                <TableRow
                  className="cursor-pointer"
                  onClick={goUp}
                >
                  <TableCell className="flex items-center gap-2 font-mono text-sm">
                    <Folder className="h-4 w-4 text-muted-foreground" />
                    ..
                  </TableCell>
                  <TableCell className="text-sm text-muted-foreground">
                    --
                  </TableCell>
                  <TableCell className="text-sm text-muted-foreground">
                    --
                  </TableCell>
                </TableRow>
              )}
              {entries.length === 0 && currentPath === "/" && (
                <TableRow>
                  <TableCell
                    colSpan={3}
                    className="text-center text-sm text-muted-foreground py-8"
                  >
                    No files found in this directory.
                  </TableCell>
                </TableRow>
              )}
              {entries.map((entry) => (
                <TableRow
                  key={entry.name}
                  className="cursor-pointer"
                  onClick={() => handleEntryClick(entry)}
                >
                  <TableCell className="flex items-center gap-2 font-mono text-sm">
                    {entry.isDirectory ? (
                      <Folder className="h-4 w-4 text-muted-foreground" />
                    ) : (
                      <File className="h-4 w-4 text-muted-foreground" />
                    )}
                    {entry.name}
                  </TableCell>
                  <TableCell className="text-sm text-muted-foreground font-mono">
                    {entry.isDirectory ? "--" : formatFileSize(entry.size)}
                  </TableCell>
                  <TableCell className="text-sm text-muted-foreground font-mono">
                    {entry.permissions}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      )}

      {/* File content viewer */}
      {selectedFile && (
        <div className="space-y-2">
          <div className="flex items-center justify-between">
            <p className="text-sm font-mono text-muted-foreground">
              {selectedFile}
              {fileContent && (
                <span className="ml-2 text-xs">
                  ({formatFileSize(String(fileContent.size))})
                </span>
              )}
            </p>
            <Button
              variant="ghost"
              size="sm"
              className="h-7 text-xs text-muted-foreground hover:text-foreground"
              onClick={() => setSelectedFile(null)}
            >
              <X className="mr-1 h-3 w-3" />
              Close
            </Button>
          </div>
          <div className="h-[400px] overflow-auto rounded-md border bg-neutral-950 p-4 font-mono text-sm text-neutral-200">
            {isFileLoading ? (
              <div className="flex items-center gap-2 text-neutral-500">
                <Loader2 className="h-3 w-3 animate-spin" />
                <span>Loading file content...</span>
              </div>
            ) : fileError ? (
              <p className="text-red-400">
                Failed to read file:{" "}
                {fileError instanceof Error
                  ? fileError.message
                  : String(fileError)}
              </p>
            ) : fileContent ? (
              <pre className="whitespace-pre-wrap">{fileContent.content}</pre>
            ) : (
              <p className="text-neutral-500">No content.</p>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
