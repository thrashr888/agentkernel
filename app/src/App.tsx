import { Routes, Route } from "react-router-dom";
import { AppShell } from "@/components/layout/AppShell";
import { Dashboard } from "@/pages/Dashboard";
import { Sandboxes } from "@/pages/Sandboxes";
import { SandboxDetail } from "@/pages/SandboxDetail";
import { Templates } from "@/pages/Templates";
import { Snapshots } from "@/pages/Snapshots";
import { Receipts } from "@/pages/Receipts";
import { Settings } from "@/pages/Settings";
import { AuditLog } from "@/pages/AuditLog";
import { Diagnostics } from "@/pages/Diagnostics";
import { Policy } from "@/pages/Policy";
import { PolicyLog } from "@/pages/PolicyLog";
import { Plugins } from "@/pages/Plugins";
import { Secrets } from "@/pages/Secrets";
import { Objects } from "@/pages/Objects";
import { Schedules } from "@/pages/Schedules";
import { Stores } from "@/pages/Stores";

function App() {
  return (
    <AppShell>
      <Routes>
        <Route path="/" element={<Dashboard />} />
        <Route path="/sandboxes" element={<Sandboxes />} />
        <Route path="/sandboxes/:name" element={<SandboxDetail />} />
        <Route path="/templates" element={<Templates />} />
        <Route path="/snapshots" element={<Snapshots />} />
        <Route path="/receipts" element={<Receipts />} />
        <Route path="/objects" element={<Objects />} />
        <Route path="/schedules" element={<Schedules />} />
        <Route path="/stores" element={<Stores />} />
        <Route path="/audit" element={<AuditLog />} />
        <Route path="/plugins" element={<Plugins />} />
        <Route path="/diagnostics" element={<Diagnostics />} />
        <Route path="/policy" element={<Policy />} />
        <Route path="/policy/log" element={<PolicyLog />} />
        <Route path="/secrets" element={<Secrets />} />
        <Route path="/settings" element={<Settings />} />
      </Routes>
    </AppShell>
  );
}

export default App;
