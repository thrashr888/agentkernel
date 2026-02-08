import { Link } from "react-router-dom";
import { Plus } from "lucide-react";
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
    </div>
  );
}
