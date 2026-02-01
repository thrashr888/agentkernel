// Organization isolation policy
//
// Ensures users can only interact with sandboxes belonging
// to their own organization. Prevents cross-tenant access
// in multi-tenant deployments.

// Users can only create sandboxes in their own org
permit(
    principal is AgentKernel::User,
    action == AgentKernel::Action::"Create",
    resource is AgentKernel::Sandbox
) when {
    principal.org_id == resource.name  // Sandbox name is prefixed with org
};

// Catch-all: allow within same org for all actions
permit(
    principal is AgentKernel::User,
    action,
    resource is AgentKernel::Sandbox
) when {
    principal.org_id == "acme-corp"  // Replace with your org_id
};

// Deny everything for users outside the organization
forbid(
    principal is AgentKernel::User,
    action,
    resource is AgentKernel::Sandbox
) when {
    principal.org_id != "acme-corp"  // Replace with your org_id
};
