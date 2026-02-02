// Port mapping restrictions
//
// Controls who can expose ports from sandboxes to the host network.
// PortMap is a sensitive action: it makes services inside sandboxes
// reachable from the host (or potentially the wider network).

// --- Allow admins to map any ports ---

permit(
    principal is AgentKernel::User,
    action == AgentKernel::Action::"PortMap",
    resource is AgentKernel::Sandbox
) when {
    principal.roles.contains("admin")
};

// --- Allow platform team to map ports ---

permit(
    principal is AgentKernel::User,
    action == AgentKernel::Action::"PortMap",
    resource is AgentKernel::Sandbox
) when {
    principal.roles.contains("platform")
};

// --- Deny all other port mapping ---
// (Cedar is default-deny, so this forbid is optional but explicit)

forbid(
    principal is AgentKernel::User,
    action == AgentKernel::Action::"PortMap",
    resource is AgentKernel::Sandbox
) unless {
    principal.roles.contains("admin") ||
    principal.roles.contains("platform")
};
