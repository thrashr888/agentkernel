import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useTemplates } from "@/lib/hooks/use-templates";
import { api } from "@/lib/api";
import type { CreateSandboxRequest, TemplateInfo } from "@/lib/types";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { toast } from "@/components/ui/use-toast";

export function Templates() {
  const { data: templates, isLoading, error } = useTemplates();
  const queryClient = useQueryClient();
  const [selectedTemplate, setSelectedTemplate] = useState<TemplateInfo | null>(
    null
  );
  const [sandboxName, setSandboxName] = useState("");
  const [profile, setProfile] = useState("moderate");

  const createMutation = useMutation({
    mutationFn: (req: CreateSandboxRequest) => api.createSandbox(req),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["sandboxes"] });
      setSelectedTemplate(null);
      setSandboxName("");
      setProfile("moderate");
      toast.success("Sandbox created from template");
    },
    onError: (err: unknown) => {
      toast.error(err instanceof Error ? err.message : String(err));
    },
  });

  function handleCreate() {
    if (!sandboxName.trim() || !selectedTemplate) return;
    createMutation.mutate({
      name: sandboxName.trim(),
      image: selectedTemplate.base_image,
      vcpus: selectedTemplate.vcpus,
      memory_mb: selectedTemplate.memory_mb,
      profile: profile,
      init_script: selectedTemplate.init_script,
      secret_mappings: selectedTemplate.secrets,
    });
  }

  const grouped = (templates ?? []).reduce<Record<string, TemplateInfo[]>>(
    (acc, t) => {
      const cat = t.category || "Other";
      if (!acc[cat]) acc[cat] = [];
      acc[cat].push(t);
      return acc;
    },
    {}
  );

  const categoryOrder = [
    "Agent Sandboxes",
    "Languages",
    "Browser Automation",
    "Infrastructure",
    "Datastores",
    "Specialized",
  ];
  const sortedCategories = [
    ...categoryOrder.filter((c) => grouped[c]),
    ...Object.keys(grouped).filter((c) => !categoryOrder.includes(c)),
  ];

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-3xl font-bold tracking-tight">Templates</h1>
        <p className="text-muted-foreground">
          Pre-configured sandbox templates for common use cases
        </p>
      </div>

      {isLoading ? (
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {[1, 2, 3, 4, 5, 6].map((i) => (
            <Skeleton key={i} className="h-[180px] rounded-lg" />
          ))}
        </div>
      ) : error ? (
        <Card>
          <CardContent className="pt-6">
            <p className="text-sm text-destructive">
              Failed to load templates: {error.message}
            </p>
          </CardContent>
        </Card>
      ) : templates && templates.length === 0 ? (
        <Card>
          <CardContent className="pt-6">
            <p className="text-sm text-muted-foreground">
              No templates available.
            </p>
          </CardContent>
        </Card>
      ) : (
        sortedCategories.map((category) => (
          <div key={category} className="space-y-4">
            <h2 className="text-xl font-semibold">{category}</h2>
            <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
              {grouped[category].map((template) => (
                <Card
                  key={template.name}
                  className="cursor-pointer transition-colors hover:bg-accent/50"
                  onClick={() => {
                    setSelectedTemplate(template);
                    setSandboxName("");
                  }}
                >
                  <CardHeader>
                    <CardTitle className="text-base">{template.name}</CardTitle>
                    <CardDescription>{template.description}</CardDescription>
                  </CardHeader>
                  <CardFooter>
                    <Badge variant="secondary">{template.base_image}</Badge>
                  </CardFooter>
                </Card>
              ))}
            </div>
          </div>
        ))
      )}

      <Dialog
        open={!!selectedTemplate}
        onOpenChange={(open) => {
          if (!open) {
            setSelectedTemplate(null);
            setSandboxName("");
            setProfile("moderate");
          }
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              Create from Template: {selectedTemplate?.name}
            </DialogTitle>
            <DialogDescription>
              {selectedTemplate?.description}
            </DialogDescription>
          </DialogHeader>
          <div className="grid gap-4 py-4">
            <div className="grid gap-2">
              <Label htmlFor="template-sandbox-name">Sandbox Name</Label>
              <Input
                id="template-sandbox-name"
                placeholder="my-sandbox"
                value={sandboxName}
                onChange={(e) => setSandboxName(e.target.value)}
              />
            </div>
            <div className="grid grid-cols-3 gap-4 text-sm">
              <div>
                <span className="text-muted-foreground">Image:</span>{" "}
                <span className="font-mono">
                  {selectedTemplate?.base_image}
                </span>
              </div>
              <div>
                <span className="text-muted-foreground">vCPUs:</span>{" "}
                {selectedTemplate?.vcpus}
              </div>
              <div>
                <span className="text-muted-foreground">Memory:</span>{" "}
                {selectedTemplate?.memory_mb} MB
              </div>
            </div>
            <div className="grid gap-2">
              <Label>Security Profile</Label>
              <Select value={profile} onValueChange={setProfile}>
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="permissive">Permissive</SelectItem>
                  <SelectItem value="moderate">Moderate</SelectItem>
                  <SelectItem value="restrictive">Restrictive</SelectItem>
                </SelectContent>
              </Select>
              <p className="text-xs text-muted-foreground">
                {profile === "permissive" && "Network + mount cwd + mount home + pass env vars"}
                {profile === "moderate" && "Network only — no mounts, no env pass-through"}
                {profile === "restrictive" && "No network, no mounts, read-only filesystem"}
              </p>
            </div>
            {selectedTemplate?.secrets && Object.keys(selectedTemplate.secrets).length > 0 && (
              <div className="grid gap-2">
                <Label>Secrets (from host environment)</Label>
                <div className="rounded-md border p-3 space-y-1">
                  {Object.entries(selectedTemplate.secrets).map(([envVar, host]) => (
                    <div key={envVar} className="flex items-center justify-between text-xs">
                      <code className="font-mono">{envVar}</code>
                      <span className="text-muted-foreground">{host}</span>
                    </div>
                  ))}
                </div>
                <p className="text-xs text-muted-foreground">
                  These environment variables will be read from your host and injected via the secure proxy.
                </p>
              </div>
            )}
          </div>
          {!!createMutation.error && (
            <p className="text-sm text-destructive">
              {createMutation.error instanceof Error ? createMutation.error.message : String(createMutation.error)}
            </p>
          )}
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setSelectedTemplate(null)}
            >
              Cancel
            </Button>
            <Button
              onClick={handleCreate}
              disabled={!sandboxName.trim() || createMutation.isPending}
            >
              {createMutation.isPending ? "Creating..." : "Create Sandbox"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
