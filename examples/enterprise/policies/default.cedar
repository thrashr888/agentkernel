// Default enterprise policy
//
// Allows all authenticated users to create and run sandboxes.
// This is a good starting point — layer restrictions on top.

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

permit(
    principal is AgentKernel::User,
    action == AgentKernel::Action::"Attach",
    resource is AgentKernel::Sandbox
);
