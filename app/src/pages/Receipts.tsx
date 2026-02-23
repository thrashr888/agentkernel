import { useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { Copy, FileCheck2, ShieldCheck } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { toast } from "@/components/ui/use-toast";

function quotePath(path: string): string {
  const trimmed = path.trim();
  if (!trimmed) return "./receipt.json";
  return trimmed.includes(" ") ? `"${trimmed.replace(/"/g, '\\"')}"` : trimmed;
}

export function Receipts() {
  const [receiptPath, setReceiptPath] = useState("./run-receipt.json");
  const [legacyPath, setLegacyPath] = useState("./legacy-receipt.json");

  const verifyCmd = useMemo(
    () => `agentkernel receipt verify ${quotePath(receiptPath)}`,
    [receiptPath]
  );
  const replayCmd = useMemo(
    () => `agentkernel receipt replay ${quotePath(receiptPath)}`,
    [receiptPath]
  );
  const verifyLegacyCmd = useMemo(
    () => `agentkernel receipt verify --allow-unsigned ${quotePath(legacyPath)}`,
    [legacyPath]
  );

  async function copy(text: string, label: string) {
    try {
      await navigator.clipboard.writeText(text);
      toast.success(`${label} copied`);
    } catch (err) {
      toast.error(err instanceof Error ? err.message : String(err));
    }
  }

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-3xl font-bold tracking-tight">Execution Receipts</h1>
        <p className="text-muted-foreground">
          Signed receipts make command execution verifiable and replayable.
        </p>
      </div>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <ShieldCheck className="h-5 w-5 text-green-600" />
            Verify Signed Receipt
          </CardTitle>
          <CardDescription>
            Verifies payload integrity and Ed25519 signature.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="grid gap-2">
            <Label htmlFor="receipt-path">Receipt file path</Label>
            <Input
              id="receipt-path"
              value={receiptPath}
              onChange={(e) => setReceiptPath(e.target.value)}
              className="font-mono"
              placeholder="./run-receipt.json"
            />
          </div>
          <div className="rounded-md border bg-muted/40 p-3">
            <code className="text-xs sm:text-sm">{verifyCmd}</code>
          </div>
          <div className="flex flex-wrap gap-2">
            <Button variant="outline" onClick={() => copy(verifyCmd, "Verify command")}>
              <Copy className="mr-2 h-4 w-4" />
              Copy Verify Command
            </Button>
            <Button variant="outline" onClick={() => copy(replayCmd, "Replay command")}>
              <FileCheck2 className="mr-2 h-4 w-4" />
              Copy Replay Command
            </Button>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Legacy Receipt Compatibility</CardTitle>
          <CardDescription>
            Use this only for older unsigned receipts generated before signing was introduced.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="grid gap-2">
            <Label htmlFor="legacy-path">Legacy receipt path</Label>
            <Input
              id="legacy-path"
              value={legacyPath}
              onChange={(e) => setLegacyPath(e.target.value)}
              className="font-mono"
              placeholder="./legacy-receipt.json"
            />
          </div>
          <div className="rounded-md border bg-muted/40 p-3">
            <code className="text-xs sm:text-sm">{verifyLegacyCmd}</code>
          </div>
          <Button
            variant="outline"
            onClick={() => copy(verifyLegacyCmd, "Legacy verify command")}
          >
            <Copy className="mr-2 h-4 w-4" />
            Copy Legacy Verify Command
          </Button>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Generate New Receipts</CardTitle>
          <CardDescription>
            Emit receipts directly from `run` and `exec`, then verify or replay them.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-2">
          <div className="rounded-md border bg-muted/40 p-3">
            <code className="text-xs sm:text-sm">
              agentkernel run --receipt ./run-receipt.json -- python3 -c "print('ok')"
            </code>
          </div>
          <div className="rounded-md border bg-muted/40 p-3">
            <code className="text-xs sm:text-sm">
              agentkernel exec my-sandbox --receipt ./exec-receipt.json -- pytest -q
            </code>
          </div>
          <p className="text-sm text-muted-foreground">
            Snapshot restore and execution receipts complement each other.
            Use{" "}
            <Link to="/snapshots" className="text-primary hover:underline">
              Snapshots
            </Link>{" "}
            for filesystem state, and Receipts for command-level provenance.
          </p>
        </CardContent>
      </Card>
    </div>
  );
}

