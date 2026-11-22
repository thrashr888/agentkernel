import { useState } from "react";
import { useIsMutating, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle, DialogTrigger } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import type { PreparedRexSandbox } from "@/lib/types";
import { api } from "@/lib/api";
import { useSettings } from "@/lib/hooks/use-settings";

function validManualRexUrl(url: string): boolean {
  if (!url.startsWith("unix:///")) return false;
  const path = url.slice("unix://".length);
  return path.length <= 103 && path.slice(1).split("/").every(
    (segment) => segment !== "" && segment !== "." && segment !== ".." && !/[^A-Za-z0-9._-]/.test(segment),
  );
}

async function copyManualRexUrl(url: string) {
  if (!validManualRexUrl(url) || !navigator.clipboard?.writeText) {
    throw new Error("Clipboard unavailable. Select the address to copy it.");
  }
  await navigator.clipboard.writeText(url);
}

export function CreateRexSandbox() {
  const [openScope, setOpenScope] = useState<string | null>(null);
  const { scope, saving, available } = usePreparedRexScope();
  if (!scope || saving || (!available && openScope !== scope)) return null;
  return <RexCreateDialog key={scope} available={available}
    onVisibilityChange={(visible) => setOpenScope(visible ? scope : null)} />;
}

function usePreparedRexScope() {
  const { settings } = useSettings();
  const saving = useIsMutating({ mutationKey: ["save-settings"] }) > 0;
  const server = settings?.servers.find((entry) => entry.name === settings.active_server);
  const scope = server ? JSON.stringify([server.name, server.url, server.config_path]) : null;
  const availability = useQuery({
    queryKey: ["local-rex-availability", scope],
    queryFn: api.localRexAvailable,
    enabled: scope !== null && !saving,
    refetchOnWindowFocus: true,
    refetchInterval: 2000,
    retry: false,
  });
  const available = !availability.isError && availability.data === true;
  return { scope, saving, available };
}

function RexCreateDialog({ available, onVisibilityChange }: {
  available: boolean;
  onVisibilityChange: (open: boolean) => void;
}) {
  const queryClient = useQueryClient();
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const [copyStatus, setCopyStatus] = useState("");
  const create = useMutation({
    retry: false,
    mutationFn: async (requestedName: string) => {
      const result = await api.createLocalRex(requestedName);
      if (result.name !== requestedName || !validManualRexUrl(result.rex_url)) {
        throw new Error("Server did not return a valid local Rex sandbox address.");
      }
      return result;
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["prepared-rex-sandboxes"] });
    },
  });
  const address = create.data?.rex_url;
  const trimmed = name.trim();
  const knownName = /^[A-Za-z0-9](?:[A-Za-z0-9_-]*[A-Za-z0-9])?$/.test(trimmed)
    && !/[-_]{2}/.test(trimmed);
  return (
    <Dialog open={open} onOpenChange={(next) => {
      if (create.isPending) return;
      setOpen(next);
      onVisibilityChange(next);
      if (!next) {
        create.reset();
        setName("");
        setCopyStatus("");
      }
    }}>
      {available && (
        <DialogTrigger asChild><Button variant="outline" className="ml-auto">Create Rex Sandbox</Button></DialogTrigger>
      )}
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Create Rex Sandbox</DialogTitle>
          <DialogDescription>Create a prepared sandbox for Rex on this Mac.</DialogDescription>
        </DialogHeader>
        {address ? (
          <div className="space-y-3">
            <p className="text-sm">Paste this address into <strong>Add Remote Host</strong> in Rex.</p>
            <code className="block break-all select-text">{address}</code>
            <Button type="button" variant="outline" onClick={() => {
              void copyManualRexUrl(address).then(
                () => setCopyStatus("Rex address copied."),
                () => setCopyStatus("Couldn’t copy. Select the address to copy it."),
              );
            }}>Copy Rex Address</Button>
            {copyStatus && <p className="text-sm" role="status">{copyStatus}</p>}
          </div>
        ) : (
          <form className="space-y-3" onSubmit={(event) => {
            event.preventDefault();
            if (available && knownName && !create.isPending) create.mutate(trimmed);
          }}>
            <Label htmlFor="rex-sandbox-name">Name</Label>
            <Input id="rex-sandbox-name" value={name} maxLength={63} autoComplete="off"
              disabled={create.isPending} onChange={(event) => setName(event.target.value)} />
            {trimmed && !knownName && (
              <p className="text-xs text-muted-foreground">Use letters or numbers, with single hyphens or underscores between them.</p>
            )}
            {create.isPending && <p className="text-sm" role="status">Starting the prepared sandbox…</p>}
            {create.isError && <p className="text-sm text-destructive" role="alert">No Rex address was returned. {String(create.error)}</p>}
            {!available && !create.isPending && !create.isError && (
              <p className="text-sm" role="status">The local Rex service is unavailable.</p>
            )}
            <Button type="submit" disabled={!available || !knownName || create.isPending}>
              {create.isPending ? "Creating…" : "Create Rex Sandbox"}
            </Button>
          </form>
        )}
      </DialogContent>
    </Dialog>
  );
}

export function PreparedRexSandboxes() {
  const { scope, saving, available } = usePreparedRexScope();
  if (!scope || saving) return null;
  return <PreparedRexList key={scope} scope={scope} available={available} />;
}

function PreparedRexList({ scope, available }: { scope: string; available: boolean }) {
  const queryKey = ["prepared-rex-sandboxes", scope];
  const list = useQuery({
    queryKey,
    queryFn: api.listPreparedRex,
    enabled: available,
    retry: false,
    refetchOnWindowFocus: true,
    refetchInterval: 5000,
  });
  if (!list.data?.length) {
    return available && list.isError
      ? <p role="status" className="text-sm text-muted-foreground">Couldn’t load prepared Rex sandboxes.</p>
      : null;
  }
  return (
    <section aria-label="Prepared Rex sandboxes" className="space-y-2 rounded-md border p-4">
      <h2 className="font-medium">Prepared Rex sandboxes</h2>
      {(!available || list.isError) && <p role="status" className="text-sm text-muted-foreground">Couldn’t refresh prepared Rex sandboxes.</p>}
      <ul className="divide-y">
        {list.data.map((box) => <PreparedRexRow key={box.name} box={box} available={available && !list.isError} />)}
      </ul>
    </section>
  );
}

function PreparedRexRow({ box, available }: { box: PreparedRexSandbox; available: boolean }) {
  const client = useQueryClient();
  const [confirmRemove, setConfirmRemove] = useState(false);
  const [copyStatus, setCopyStatus] = useState("");
  const refresh = () => void client.invalidateQueries({ queryKey: ["prepared-rex-sandboxes"] });
  const reopen = useMutation({
    retry: false,
    mutationFn: async () => {
      const result = await api.reopenPreparedRex(box.name);
      if (result.name !== box.name || !validManualRexUrl(result.rex_url)) {
        throw new Error("No verified Rex address was returned.");
      }
      return result;
    },
    onSettled: refresh,
  });
  const remove = useMutation({
    retry: false,
    mutationFn: () => api.removePreparedRex(box.name),
    onSuccess: () => {
      reopen.reset();
      setCopyStatus("");
      setConfirmRemove(false);
    },
    onSettled: refresh,
  });
  const unavailable = !available || box.status === "unavailable";
  const address = !unavailable && !remove.isPending && !remove.isSuccess ? reopen.data?.rex_url : undefined;
  return (
    <li className="space-y-2 py-2">
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-medium">{box.name}</span>
        <span className="text-xs text-muted-foreground">
          {box.status === "open" ? "Open" : box.status === "reopenable" ? "Can reopen" : "Unavailable"}
        </span>
        {!remove.isSuccess && (
          <>
            <Button type="button" size="sm" variant="outline"
              disabled={unavailable || reopen.isPending || remove.isPending || confirmRemove}
              onClick={() => { setCopyStatus(""); reopen.mutate(); }}>
              {reopen.isPending ? "Checking…" : "Get Rex Address"}
            </Button>
            {confirmRemove ? (
              <>
                <span className="text-sm">Stop and remove this sandbox?</span>
                <Button type="button" size="sm" variant="destructive" disabled={!available || remove.isPending}
                  onClick={() => remove.mutate()}>Confirm removal</Button>
                <Button type="button" size="sm" variant="outline" disabled={remove.isPending}
                  onClick={() => setConfirmRemove(false)}>Cancel</Button>
              </>
            ) : (
              <Button type="button" size="sm" variant="outline" disabled={!available || reopen.isPending}
                onClick={() => setConfirmRemove(true)}>Remove</Button>
            )}
          </>
        )}
      </div>
      {address && (
        <div className="space-y-1">
          <code className="block break-all select-text">{address}</code>
          <Button type="button" size="sm" variant="outline" onClick={() => {
            void copyManualRexUrl(address).then(
              () => setCopyStatus("Rex address copied."),
              () => setCopyStatus("Couldn’t copy. Select the address to copy it."),
            );
          }}>Copy Rex Address</Button>
          {copyStatus && <p className="text-sm" role="status">{copyStatus}</p>}
        </div>
      )}
      {reopen.isError && !remove.isSuccess && <p role="alert" className="text-sm">Couldn’t verify the original Rex sandbox.</p>}
      {remove.isError && <p role="alert" className="text-sm">Couldn’t confirm removal. Check the sandbox before retrying.</p>}
    </li>
  );
}
