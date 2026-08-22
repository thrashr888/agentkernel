import { useState } from "react";
import { Link } from "react-router-dom";
import { Plus, Play, Loader2 } from "lucide-react";
import { useMutation } from "@tanstack/react-query";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogTrigger,
} from "@/components/ui/dialog";
import { toast } from "@/components/ui/use-toast";
import { api } from "@/lib/api";
import type { RunOutput } from "@/lib/types";

export function QuickActions() {
  const [open, setOpen] = useState(false);
  const [command, setCommand] = useState("");
  const [image, setImage] = useState("alpine:3.24");
  const [output, setOutput] = useState<string | null>(null);

  const quickRunMutation = useMutation({
    mutationFn: ({
      command,
      image,
      profile,
    }: {
      command: string[];
      image?: string;
      profile?: string;
    }) => api.quickRun(command, image, profile),
    onSuccess: (data: RunOutput) => {
      setOutput(data.output);
    },
    onError: (err: unknown) => {
      toast.error(err instanceof Error ? err.message : String(err));
    },
  });

  function handleRun() {
    const trimmed = command.trim();
    if (!trimmed) {
      toast.error("Command is required");
      return;
    }
    // Split the command string into args (shell-style split on whitespace)
    const args = trimmed.split(/\s+/);
    setOutput(null);
    quickRunMutation.mutate({
      command: args,
      image: image || undefined,
      profile: "moderate",
    });
  }

  function handleOpenChange(next: boolean) {
    setOpen(next);
    if (!next) {
      // Reset state when closing
      setCommand("");
      setImage("alpine:3.24");
      setOutput(null);
      quickRunMutation.reset();
    }
  }

  return (
    <div className="flex flex-wrap gap-3">
      <Button asChild>
        <Link to="/sandboxes">
          <Plus className="mr-2 h-4 w-4" />
          Create Sandbox
        </Link>
      </Button>

      <Dialog open={open} onOpenChange={handleOpenChange}>
        <DialogTrigger asChild>
          <Button variant="outline">
            <Play className="mr-2 h-4 w-4" />
            Quick Run
          </Button>
        </DialogTrigger>
        <DialogContent className="sm:max-w-lg">
          <DialogHeader>
            <DialogTitle>Quick Run</DialogTitle>
            <DialogDescription>
              Execute a command in a temporary sandbox. The sandbox is created,
              the command runs, and it is cleaned up automatically.
            </DialogDescription>
          </DialogHeader>

          <div className="grid gap-4 py-2">
            <div className="grid gap-2">
              <Label htmlFor="qr-image">Image</Label>
              <Input
                id="qr-image"
                placeholder="alpine:3.24"
                value={image}
                onChange={(e) => setImage(e.target.value)}
                disabled={quickRunMutation.isPending}
              />
            </div>

            <div className="grid gap-2">
              <Label htmlFor="qr-command">Command</Label>
              <Input
                id="qr-command"
                placeholder="echo hello world"
                value={command}
                onChange={(e) => setCommand(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && !quickRunMutation.isPending) {
                    handleRun();
                  }
                }}
                disabled={quickRunMutation.isPending}
                autoFocus
              />
            </div>

            <Button
              onClick={handleRun}
              disabled={quickRunMutation.isPending || !command.trim()}
            >
              {quickRunMutation.isPending ? (
                <>
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  Running...
                </>
              ) : (
                <>
                  <Play className="mr-2 h-4 w-4" />
                  Run
                </>
              )}
            </Button>

            {output !== null && (
              <div className="grid gap-2">
                <Label>Output</Label>
                <pre className="max-h-64 overflow-auto rounded-md bg-neutral-950 p-4 font-mono text-xs text-neutral-200 whitespace-pre-wrap">
                  {output || "(no output)"}
                </pre>
              </div>
            )}
          </div>
        </DialogContent>
      </Dialog>
    </div>
  );
}
