// Role-based access control
//
// Developers can create, run, and exec sandboxes.
// Admins can additionally mount volumes and enable networking.
// Viewers can only attach (read-only observe).

// --- Developer role ---

permit(
    principal is AgentKernel::User,
    action == AgentKernel::Action::"Create",
    resource is AgentKernel::Sandbox
) when {
    principal.roles.contains("developer")
};

permit(
    principal is AgentKernel::User,
    action == AgentKernel::Action::"Run",
    resource is AgentKernel::Sandbox
) when {
    principal.roles.contains("developer")
};

permit(
    principal is AgentKernel::User,
    action == AgentKernel::Action::"Exec",
    resource is AgentKernel::Sandbox
) when {
    principal.roles.contains("developer")
};

permit(
    principal is AgentKernel::User,
    action == AgentKernel::Action::"Attach",
    resource is AgentKernel::Sandbox
) when {
    principal.roles.contains("developer")
};

// --- Admin role (all actions) ---

permit(
    principal is AgentKernel::User,
    action == AgentKernel::Action::"Mount",
    resource is AgentKernel::Sandbox
) when {
    principal.roles.contains("admin")
};

permit(
    principal is AgentKernel::User,
    action == AgentKernel::Action::"Network",
    resource is AgentKernel::Sandbox
) when {
    principal.roles.contains("admin")
};

// Admins inherit all developer permissions too
permit(
    principal is AgentKernel::User,
    action,
    resource is AgentKernel::Sandbox
) when {
    principal.roles.contains("admin")
};

// --- Viewer role (attach only) ---

permit(
    principal is AgentKernel::User,
    action == AgentKernel::Action::"Attach",
    resource is AgentKernel::Sandbox
) when {
    principal.roles.contains("viewer")
};
