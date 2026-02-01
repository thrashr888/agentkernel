// Runtime and agent restrictions
//
// Controls which runtimes and agent types users can access.
// Useful for limiting blast radius or enforcing approved toolchains.

// Allow Python and Node runtimes for all developers
permit(
    principal is AgentKernel::User,
    action == AgentKernel::Action::"Create",
    resource is AgentKernel::Sandbox
) when {
    principal.roles.contains("developer") &&
    (resource.runtime == "python" || resource.runtime == "node")
};

permit(
    principal is AgentKernel::User,
    action == AgentKernel::Action::"Run",
    resource is AgentKernel::Sandbox
) when {
    principal.roles.contains("developer") &&
    (resource.runtime == "python" || resource.runtime == "node")
};

// Rust and Go runtimes require the "platform" role
permit(
    principal is AgentKernel::User,
    action == AgentKernel::Action::"Create",
    resource is AgentKernel::Sandbox
) when {
    principal.roles.contains("platform") &&
    (resource.runtime == "rust" || resource.runtime == "go")
};

permit(
    principal is AgentKernel::User,
    action == AgentKernel::Action::"Run",
    resource is AgentKernel::Sandbox
) when {
    principal.roles.contains("platform") &&
    (resource.runtime == "rust" || resource.runtime == "go")
};

// Only allow Claude and Gemini agents (block Codex for compliance reasons)
forbid(
    principal is AgentKernel::User,
    action,
    resource is AgentKernel::Sandbox
) when {
    resource.agent_type == "codex"
};
