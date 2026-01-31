//! Cedar policy evaluation engine for enterprise authorization.
//!
//! Defines the Cedar schema for the AgentKernel namespace and provides
//! policy evaluation for sandbox operations (Create, Run, Exec, Attach,
//! Mount, Network).

#![cfg(feature = "enterprise")]

use anyhow::{Context as _, Result, bail};
use cedar_policy::{
    Authorizer, Context, Decision, Entities, Entity, EntityTypeName, EntityUid, PolicySet, Request,
    RestrictedExpression, Schema,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::str::FromStr;

/// Cedar schema definition for the AgentKernel namespace.
///
/// Defines entity types (User, Sandbox) and actions
/// (Run, Exec, Create, Attach, Mount, Network).
pub const CEDAR_SCHEMA: &str = r#"
namespace AgentKernel {
    entity User = {
        email: String,
        org_id: String,
        roles: Set<String>,
        mfa_verified: Bool,
    };

    entity Sandbox = {
        name: String,
        agent_type: String,
        runtime: String,
    };

    action Run appliesTo {
        principal: [User],
        resource: [Sandbox],
    };

    action Exec appliesTo {
        principal: [User],
        resource: [Sandbox],
    };

    action Create appliesTo {
        principal: [User],
        resource: [Sandbox],
    };

    action Attach appliesTo {
        principal: [User],
        resource: [Sandbox],
    };

    action Mount appliesTo {
        principal: [User],
        resource: [Sandbox],
    };

    action Network appliesTo {
        principal: [User],
        resource: [Sandbox],
    };
}
"#;

/// Actions supported by the AgentKernel policy schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Action {
    Run,
    Exec,
    Create,
    Attach,
    Mount,
    Network,
}

impl Action {
    /// Get the Cedar entity UID string for this action.
    pub fn cedar_uid(&self) -> String {
        match self {
            Action::Run => r#"AgentKernel::Action::"Run""#.to_string(),
            Action::Exec => r#"AgentKernel::Action::"Exec""#.to_string(),
            Action::Create => r#"AgentKernel::Action::"Create""#.to_string(),
            Action::Attach => r#"AgentKernel::Action::"Attach""#.to_string(),
            Action::Mount => r#"AgentKernel::Action::"Mount""#.to_string(),
            Action::Network => r#"AgentKernel::Action::"Network""#.to_string(),
        }
    }
}

impl std::fmt::Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Action::Run => write!(f, "Run"),
            Action::Exec => write!(f, "Exec"),
            Action::Create => write!(f, "Create"),
            Action::Attach => write!(f, "Attach"),
            Action::Mount => write!(f, "Mount"),
            Action::Network => write!(f, "Network"),
        }
    }
}

/// Principal identity for policy evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Principal {
    /// User identifier (e.g., email)
    pub id: String,
    /// Email address
    pub email: String,
    /// Organization identifier
    pub org_id: String,
    /// Assigned roles
    pub roles: Vec<String>,
    /// Whether MFA has been verified
    pub mfa_verified: bool,
}

/// Resource (sandbox) for policy evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resource {
    /// Sandbox name
    pub name: String,
    /// Agent type (claude, gemini, codex, opencode)
    pub agent_type: String,
    /// Runtime (python, node, rust, etc.)
    pub runtime: String,
}

/// Result of a policy evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyDecision {
    /// Whether the action is permitted
    pub decision: PolicyEffect,
    /// Reason for the decision
    pub reason: String,
    /// IDs of policies that contributed to this decision
    pub matched_policies: Vec<String>,
    /// Evaluation duration in microseconds
    pub evaluation_time_us: u64,
}

/// The effect of a policy decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyEffect {
    /// Action is allowed
    Permit,
    /// Action is denied
    Deny,
}

impl PolicyDecision {
    /// Create a permit decision.
    pub fn permit(reason: impl Into<String>, matched: Vec<String>, time_us: u64) -> Self {
        Self {
            decision: PolicyEffect::Permit,
            reason: reason.into(),
            matched_policies: matched,
            evaluation_time_us: time_us,
        }
    }

    /// Create a deny decision.
    pub fn deny(reason: impl Into<String>, matched: Vec<String>, time_us: u64) -> Self {
        Self {
            decision: PolicyEffect::Deny,
            reason: reason.into(),
            matched_policies: matched,
            evaluation_time_us: time_us,
        }
    }

    /// Check if the decision permits the action.
    pub fn is_permit(&self) -> bool {
        self.decision == PolicyEffect::Permit
    }
}

/// Cedar policy evaluation engine.
///
/// Loads Cedar policies and schema, then evaluates authorization requests
/// against them.
pub struct CedarEngine {
    /// Parsed Cedar policies
    policy_set: PolicySet,
    /// Parsed Cedar schema
    schema: Schema,
    /// The authorizer instance
    authorizer: Authorizer,
}

impl CedarEngine {
    /// Create a new CedarEngine with the given policies and the built-in schema.
    pub fn new(policies_src: &str) -> Result<Self> {
        let (schema, _warnings) = Schema::from_cedarschema_str(CEDAR_SCHEMA)
            .context("Failed to parse Cedar schema")?;

        let policy_set: PolicySet = policies_src
            .parse()
            .map_err(|e| anyhow::anyhow!("Failed to parse Cedar policies: {}", e))?;

        Ok(Self {
            policy_set,
            schema,
            authorizer: Authorizer::new(),
        })
    }

    /// Create a CedarEngine with a custom schema.
    pub fn with_schema(policies_src: &str, schema_src: &str) -> Result<Self> {
        let (schema, _warnings) = Schema::from_cedarschema_str(schema_src)
            .context("Failed to parse Cedar schema")?;

        let policy_set: PolicySet = policies_src
            .parse()
            .map_err(|e| anyhow::anyhow!("Failed to parse Cedar policies: {}", e))?;

        Ok(Self {
            policy_set,
            schema,
            authorizer: Authorizer::new(),
        })
    }

    /// Replace the policy set with new policies.
    pub fn update_policies(&mut self, policies_src: &str) -> Result<()> {
        let policy_set: PolicySet = policies_src
            .parse()
            .map_err(|e| anyhow::anyhow!("Failed to parse Cedar policies: {}", e))?;
        self.policy_set = policy_set;
        Ok(())
    }

    /// Evaluate an authorization request.
    ///
    /// Returns a PolicyDecision indicating whether the principal is allowed
    /// to perform the action on the resource.
    pub fn evaluate(
        &self,
        principal: &Principal,
        action: Action,
        resource: &Resource,
        extra_context: Option<HashMap<String, String>>,
    ) -> PolicyDecision {
        let start = std::time::Instant::now();

        // Build Cedar entities
        let entities = match self.build_entities(principal, resource) {
            Ok(e) => e,
            Err(e) => {
                let elapsed = start.elapsed().as_micros() as u64;
                return PolicyDecision::deny(
                    format!("Failed to build entities: {}", e),
                    vec![],
                    elapsed,
                );
            }
        };

        // Build request
        let request = match self.build_request(principal, action, resource, extra_context) {
            Ok(r) => r,
            Err(e) => {
                let elapsed = start.elapsed().as_micros() as u64;
                return PolicyDecision::deny(
                    format!("Failed to build request: {}", e),
                    vec![],
                    elapsed,
                );
            }
        };

        // Evaluate
        let response = self
            .authorizer
            .is_authorized(&request, &self.policy_set, &entities);
        let elapsed = start.elapsed().as_micros() as u64;

        let matched: Vec<String> = response
            .diagnostics()
            .reason()
            .map(|id| id.to_string())
            .collect();

        match response.decision() {
            Decision::Allow => PolicyDecision::permit("Policy evaluation: permit", matched, elapsed),
            Decision::Deny => {
                let errors: Vec<String> = response
                    .diagnostics()
                    .errors()
                    .map(|e| e.to_string())
                    .collect();

                let reason = if errors.is_empty() {
                    "Policy evaluation: deny (no matching permit or explicit forbid)".to_string()
                } else {
                    format!("Policy evaluation: deny (errors: {})", errors.join("; "))
                };

                PolicyDecision::deny(reason, matched, elapsed)
            }
        }
    }

    /// Get a reference to the schema (for validation).
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// Build Cedar entities from principal and resource.
    fn build_entities(&self, principal: &Principal, resource: &Resource) -> Result<Entities> {
        let mut entities_vec = Vec::new();

        // Build User entity
        let user_type = EntityTypeName::from_str("AgentKernel::User")
            .map_err(|e| anyhow::anyhow!("Invalid entity type: {}", e))?;
        let user_uid = EntityUid::from_type_name_and_id(user_type, principal.id.clone().into());

        let roles_set: HashSet<RestrictedExpression> = principal
            .roles
            .iter()
            .map(|r| RestrictedExpression::new_string(r.clone()))
            .collect();

        let user_attrs: HashMap<String, RestrictedExpression> = [
            (
                "email".to_string(),
                RestrictedExpression::new_string(principal.email.clone()),
            ),
            (
                "org_id".to_string(),
                RestrictedExpression::new_string(principal.org_id.clone()),
            ),
            (
                "roles".to_string(),
                RestrictedExpression::new_set(roles_set.into_iter()),
            ),
            (
                "mfa_verified".to_string(),
                RestrictedExpression::new_bool(principal.mfa_verified),
            ),
        ]
        .into_iter()
        .collect();

        let user_entity = Entity::new(user_uid.clone(), user_attrs, HashSet::new())
            .map_err(|e| anyhow::anyhow!("Failed to create User entity: {}", e))?;
        entities_vec.push(user_entity);

        // Build Sandbox entity
        let sandbox_type = EntityTypeName::from_str("AgentKernel::Sandbox")
            .map_err(|e| anyhow::anyhow!("Invalid entity type: {}", e))?;
        let sandbox_uid =
            EntityUid::from_type_name_and_id(sandbox_type, resource.name.clone().into());

        let sandbox_attrs: HashMap<String, RestrictedExpression> = [
            (
                "name".to_string(),
                RestrictedExpression::new_string(resource.name.clone()),
            ),
            (
                "agent_type".to_string(),
                RestrictedExpression::new_string(resource.agent_type.clone()),
            ),
            (
                "runtime".to_string(),
                RestrictedExpression::new_string(resource.runtime.clone()),
            ),
        ]
        .into_iter()
        .collect();

        let sandbox_entity = Entity::new(sandbox_uid, sandbox_attrs, HashSet::new())
            .map_err(|e| anyhow::anyhow!("Failed to create Sandbox entity: {}", e))?;
        entities_vec.push(sandbox_entity);

        Entities::from_entities(entities_vec, Some(&self.schema))
            .context("Failed to build entity set")
    }

    /// Build a Cedar Request from principal, action, resource, and optional context.
    fn build_request(
        &self,
        principal: &Principal,
        action: Action,
        resource: &Resource,
        extra_context: Option<HashMap<String, String>>,
    ) -> Result<Request> {
        let principal_uid: EntityUid = format!(r#"AgentKernel::User::"{}""#, principal.id)
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid principal UID: {}", e))?;

        let action_uid: EntityUid = action
            .cedar_uid()
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid action UID: {}", e))?;

        let resource_uid: EntityUid = format!(r#"AgentKernel::Sandbox::"{}""#, resource.name)
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid resource UID: {}", e))?;

        let context = if let Some(ctx_map) = extra_context {
            Context::from_pairs(
                ctx_map
                    .into_iter()
                    .map(|(k, v)| (k, RestrictedExpression::new_string(v))),
                None,
            )
            .context("Failed to build context")?
        } else {
            Context::empty()
        };

        Request::new(
            principal_uid,
            action_uid,
            resource_uid,
            context,
            Some(&self.schema),
        )
        .map_err(|e| anyhow::anyhow!("Failed to create request: {}", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_principal() -> Principal {
        Principal {
            id: "alice".to_string(),
            email: "alice@acme.com".to_string(),
            org_id: "acme-corp".to_string(),
            roles: vec!["developer".to_string()],
            mfa_verified: true,
        }
    }

    fn test_resource() -> Resource {
        Resource {
            name: "my-sandbox".to_string(),
            agent_type: "claude".to_string(),
            runtime: "python".to_string(),
        }
    }

    #[test]
    fn test_permit_policy() {
        let policies = r#"
permit(
    principal is AgentKernel::User,
    action == AgentKernel::Action::"Run",
    resource is AgentKernel::Sandbox
);
        "#;

        let engine = CedarEngine::new(policies).unwrap();
        let decision = engine.evaluate(&test_principal(), Action::Run, &test_resource(), None);

        assert!(decision.is_permit());
        assert_eq!(decision.decision, PolicyEffect::Permit);
    }

    #[test]
    fn test_deny_no_matching_policy() {
        // No policies match Exec action
        let policies = r#"
permit(
    principal is AgentKernel::User,
    action == AgentKernel::Action::"Run",
    resource is AgentKernel::Sandbox
);
        "#;

        let engine = CedarEngine::new(policies).unwrap();
        let decision = engine.evaluate(&test_principal(), Action::Exec, &test_resource(), None);

        assert!(!decision.is_permit());
        assert_eq!(decision.decision, PolicyEffect::Deny);
    }

    #[test]
    fn test_explicit_forbid() {
        let policies = r#"
permit(
    principal is AgentKernel::User,
    action == AgentKernel::Action::"Network",
    resource is AgentKernel::Sandbox
);
forbid(
    principal is AgentKernel::User,
    action == AgentKernel::Action::"Network",
    resource is AgentKernel::Sandbox
) when {
    !principal.mfa_verified
};
        "#;

        // MFA verified user should be permitted (forbid doesn't match)
        let engine = CedarEngine::new(policies).unwrap();
        let decision = engine.evaluate(&test_principal(), Action::Network, &test_resource(), None);
        assert!(decision.is_permit());

        // Non-MFA user should be denied
        let mut no_mfa = test_principal();
        no_mfa.mfa_verified = false;
        let decision = engine.evaluate(&no_mfa, Action::Network, &test_resource(), None);
        assert!(!decision.is_permit());
    }

    #[test]
    fn test_role_based_policy() {
        let policies = r#"
permit(
    principal is AgentKernel::User,
    action == AgentKernel::Action::"Create",
    resource is AgentKernel::Sandbox
) when {
    principal.roles.contains("developer")
};
        "#;

        let engine = CedarEngine::new(policies).unwrap();

        // Developer should be permitted
        let decision = engine.evaluate(&test_principal(), Action::Create, &test_resource(), None);
        assert!(decision.is_permit());

        // Non-developer should be denied
        let mut viewer = test_principal();
        viewer.roles = vec!["viewer".to_string()];
        let decision = engine.evaluate(&viewer, Action::Create, &test_resource(), None);
        assert!(!decision.is_permit());
    }

    #[test]
    fn test_update_policies() {
        let initial = r#"
permit(
    principal is AgentKernel::User,
    action == AgentKernel::Action::"Run",
    resource is AgentKernel::Sandbox
);
        "#;

        let mut engine = CedarEngine::new(initial).unwrap();

        // Initially permits Run
        let decision = engine.evaluate(&test_principal(), Action::Run, &test_resource(), None);
        assert!(decision.is_permit());

        // Update to only permit Create
        let updated = r#"
permit(
    principal is AgentKernel::User,
    action == AgentKernel::Action::"Create",
    resource is AgentKernel::Sandbox
);
        "#;
        engine.update_policies(updated).unwrap();

        // Run should now be denied
        let decision = engine.evaluate(&test_principal(), Action::Run, &test_resource(), None);
        assert!(!decision.is_permit());

        // Create should be permitted
        let decision = engine.evaluate(&test_principal(), Action::Create, &test_resource(), None);
        assert!(decision.is_permit());
    }

    #[test]
    fn test_action_display() {
        assert_eq!(Action::Run.to_string(), "Run");
        assert_eq!(Action::Exec.to_string(), "Exec");
        assert_eq!(Action::Create.to_string(), "Create");
        assert_eq!(Action::Attach.to_string(), "Attach");
        assert_eq!(Action::Mount.to_string(), "Mount");
        assert_eq!(Action::Network.to_string(), "Network");
    }

    #[test]
    fn test_policy_decision_helpers() {
        let permit = PolicyDecision::permit("ok", vec!["policy0".to_string()], 100);
        assert!(permit.is_permit());
        assert_eq!(permit.matched_policies, vec!["policy0"]);
        assert_eq!(permit.evaluation_time_us, 100);

        let deny = PolicyDecision::deny("nope", vec![], 50);
        assert!(!deny.is_permit());
    }

    #[test]
    fn test_empty_policies() {
        // Empty policy set should deny everything (default deny)
        let engine = CedarEngine::new("").unwrap();
        let decision = engine.evaluate(&test_principal(), Action::Run, &test_resource(), None);
        assert!(!decision.is_permit());
    }
}
