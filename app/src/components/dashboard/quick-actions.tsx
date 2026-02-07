import { Link } from "react-router-dom";
import { Plus, Layers, Camera } from "lucide-react";
import { Button } from "@/components/ui/button";

export function QuickActions() {
  return (
    <div className="flex flex-wrap gap-3">
      <Button asChild>
        <Link to="/sandboxes">
          <Plus className="mr-2 h-4 w-4" />
          Create Sandbox
        </Link>
      </Button>
      <Button variant="outline" asChild>
        <Link to="/templates">
          <Layers className="mr-2 h-4 w-4" />
          Browse Templates
        </Link>
      </Button>
      <Button variant="outline" asChild>
        <Link to="/snapshots">
          <Camera className="mr-2 h-4 w-4" />
          View Snapshots
        </Link>
      </Button>
    </div>
  );
}
