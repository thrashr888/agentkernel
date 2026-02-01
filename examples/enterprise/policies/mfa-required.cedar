// MFA enforcement policy
//
// Requires MFA verification for sensitive operations:
// network access and volume mounts. These actions expose
// the sandbox to external systems or host data.

// Allow basic sandbox operations for all authenticated users
permit(
    principal is AgentKernel::User,
    action == AgentKernel::Action::"Create",
    resource is AgentKernel::Sandbox
);

permit(
    principal is AgentKernel::User,
    action == AgentKernel::Action::"Run",
    resource is AgentKernel::Sandbox
);

permit(
    principal is AgentKernel::User,
    action == AgentKernel::Action::"Exec",
    resource is AgentKernel::Sandbox
);

// Network access requires MFA
permit(
    principal is AgentKernel::User,
    action == AgentKernel::Action::"Network",
    resource is AgentKernel::Sandbox
) when {
    principal.mfa_verified
};

// Deny network access without MFA (explicit forbid overrides permit)
forbid(
    principal is AgentKernel::User,
    action == AgentKernel::Action::"Network",
    resource is AgentKernel::Sandbox
) when {
    !principal.mfa_verified
};

// Mount requires MFA
permit(
    principal is AgentKernel::User,
    action == AgentKernel::Action::"Mount",
    resource is AgentKernel::Sandbox
) when {
    principal.mfa_verified
};

forbid(
    principal is AgentKernel::User,
    action == AgentKernel::Action::"Mount",
    resource is AgentKernel::Sandbox
) when {
    !principal.mfa_verified
};
