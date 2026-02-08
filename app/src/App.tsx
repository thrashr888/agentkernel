import { Routes, Route } from "react-router-dom";
import { AppShell } from "@/components/layout/AppShell";
import { Dashboard } from "@/pages/Dashboard";
import { Sandboxes } from "@/pages/Sandboxes";
import { SandboxDetail } from "@/pages/SandboxDetail";
import { Templates } from "@/pages/Templates";
import { Snapshots } from "@/pages/Snapshots";
import { Settings } from "@/pages/Settings";
import { Diagnostics } from "@/pages/Diagnostics";

function App() {
  return (
    <AppShell>
      <Routes>
        <Route path="/" element={<Dashboard />} />
        <Route path="/sandboxes" element={<Sandboxes />} />
        <Route path="/sandboxes/:name" element={<SandboxDetail />} />
        <Route path="/templates" element={<Templates />} />
        <Route path="/snapshots" element={<Snapshots />} />
        <Route path="/diagnostics" element={<Diagnostics />} />
        <Route path="/settings" element={<Settings />} />
      </Routes>
    </AppShell>
  );
}

export default App;
