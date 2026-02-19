import { useState, useRef, useEffect } from "react";
import { ChevronsUpDown, Check, X, Plus, Loader2 } from "lucide-react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useSandboxes } from "@/lib/hooks/use-sandboxes";
import { api } from "@/lib/api";
import { toast } from "@/components/ui/use-toast";
import { cn } from "@/lib/utils";

interface SandboxPickerProps {
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  optional?: boolean;
}

export function SandboxPicker({
  value,
  onChange,
  placeholder = "Select a sandbox...",
  optional = true,
}: SandboxPickerProps) {
  const queryClient = useQueryClient();
  const { data: sandboxes } = useSandboxes();
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState("");
  const [creatingName, setCreatingName] = useState<string | null>(null);
  const [newImage, setNewImage] = useState("alpine:3.20");
  const containerRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  // Close on outside click
  useEffect(() => {
    function handleClick(e: MouseEvent) {
      if (
        containerRef.current &&
        !containerRef.current.contains(e.target as Node)
      ) {
        setOpen(false);
        setCreatingName(null);
      }
    }
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, []);

  const names = (sandboxes?.map((s) => s.name) ?? []).sort((a, b) =>
    a.localeCompare(b),
  );

  const filtered = names.filter((n) =>
    n.toLowerCase().includes(search.toLowerCase()),
  );

  const trimmedSearch = search.trim();
  const exactMatch = names.some(
    (n) => n.toLowerCase() === trimmedSearch.toLowerCase(),
  );
  const showCreate = trimmedSearch.length > 0 && !exactMatch;

  const createMutation = useMutation({
    mutationFn: (name: string) =>
      api.createSandbox({ name, image: newImage.trim() || "alpine:3.20" }),
    onSuccess: (_data, name) => {
      queryClient.invalidateQueries({ queryKey: ["sandboxes"] });
      onChange(name);
      setOpen(false);
      setSearch("");
      setCreatingName(null);
      setNewImage("alpine:3.20");
      toast.success(`Sandbox "${name}" created`);
    },
    onError: (err) => {
      toast.error(err instanceof Error ? err.message : String(err));
    },
  });

  return (
    <div ref={containerRef} className="relative">
      <div
        className={cn(
          "flex items-center rounded-md border bg-background px-3 py-2 text-sm",
          "focus-within:ring-2 focus-within:ring-ring",
          open && "ring-2 ring-ring",
        )}
      >
        <input
          ref={inputRef}
          type="text"
          className="flex-1 bg-transparent outline-none font-mono placeholder:text-muted-foreground"
          placeholder={value ? "" : placeholder}
          value={open ? search : value}
          onChange={(e) => {
            setSearch(e.target.value);
            setCreatingName(null);
            if (!open) setOpen(true);
          }}
          onFocus={() => {
            setOpen(true);
            setSearch("");
          }}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              setOpen(false);
              setCreatingName(null);
              inputRef.current?.blur();
            }
          }}
        />
        {value && optional && (
          <button
            type="button"
            onClick={() => {
              onChange("");
              setSearch("");
            }}
            className="mr-1 text-muted-foreground hover:text-foreground"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        )}
        <button
          type="button"
          onClick={() => {
            setOpen(!open);
            setCreatingName(null);
            if (!open) {
              setSearch("");
              inputRef.current?.focus();
            }
          }}
          className="text-muted-foreground hover:text-foreground"
        >
          <ChevronsUpDown className="h-3.5 w-3.5" />
        </button>
      </div>

      {open && (
        <div className="absolute z-50 mt-1 w-full rounded-md border bg-popover shadow-md">
          {/* Inline create form */}
          {creatingName && (
            <div className="border-b px-3 py-2 space-y-2">
              <p className="text-xs text-muted-foreground">
                Create sandbox <span className="font-mono font-medium text-foreground">{creatingName}</span>
              </p>
              <div className="flex gap-2">
                <input
                  type="text"
                  value={newImage}
                  onChange={(e) => setNewImage(e.target.value)}
                  placeholder="Image (e.g. alpine:3.20)"
                  className="flex-1 rounded border bg-background px-2 py-1 text-xs font-mono focus:outline-none focus:ring-1 focus:ring-ring"
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      createMutation.mutate(creatingName);
                    }
                  }}
                  autoFocus
                />
                <button
                  type="button"
                  className="rounded bg-primary px-2 py-1 text-xs text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
                  disabled={createMutation.isPending}
                  onClick={() => createMutation.mutate(creatingName)}
                >
                  {createMutation.isPending ? (
                    <Loader2 className="h-3 w-3 animate-spin" />
                  ) : (
                    "Create"
                  )}
                </button>
              </div>
            </div>
          )}

          <ul className="max-h-[200px] overflow-y-auto py-1">
            {filtered.map((name) => (
              <li key={name}>
                <button
                  type="button"
                  className={cn(
                    "flex w-full items-center gap-2 px-3 py-1.5 text-sm hover:bg-accent",
                    name === value && "bg-accent/50",
                  )}
                  onClick={() => {
                    onChange(name);
                    setOpen(false);
                    setSearch("");
                    setCreatingName(null);
                  }}
                >
                  <Check
                    className={cn(
                      "h-3.5 w-3.5 shrink-0",
                      name === value ? "opacity-100" : "opacity-0",
                    )}
                  />
                  <span className="font-mono truncate">{name}</span>
                </button>
              </li>
            ))}

            {/* Create new option */}
            {showCreate && !creatingName && (
              <li>
                <button
                  type="button"
                  className="flex w-full items-center gap-2 px-3 py-1.5 text-sm hover:bg-accent text-primary"
                  onClick={() => setCreatingName(trimmedSearch)}
                >
                  <Plus className="h-3.5 w-3.5 shrink-0" />
                  <span className="font-mono truncate">
                    Create "{trimmedSearch}"
                  </span>
                </button>
              </li>
            )}
          </ul>

          {filtered.length === 0 && !showCreate && (
            <div className="px-3 py-2 text-sm text-muted-foreground">
              No sandboxes available
            </div>
          )}
        </div>
      )}
    </div>
  );
}
