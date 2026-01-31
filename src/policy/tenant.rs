//! Multi-tenant organization and team hierarchy for enterprise policy management.
//!
//! Implements a hierarchical policy resolution model:
//! - Organization (top level)
//! - Team (within an organization)
//! - User (within a team)
//!
//! Policies are resolved from most specific (User) to least specific (Global).
//! Key invariant: `forbid` ALWAYS overrides `permit` regardless of specificity.

#[cfg(feature = "enterprise")]
use serde::{Deserialize, Serialize};

/// A policy decision: permit or forbid an action.
#[cfg(feature = "enterprise")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PolicyDecision {
    /// Allow the action
    Permit,
    /// Deny the action
    Forbid,
}

/// A named policy with a decision and optional conditions.
#[cfg(feature = "enterprise")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    /// Unique policy identifier
    pub id: String,
    /// Human-readable policy name
    pub name: String,
    /// The action this policy applies to (e.g., "Run", "Network", "Mount")
    pub action: String,
    /// Whether this policy permits or forbids the action
    pub decision: PolicyDecision,
    /// Priority within its scope (higher = evaluated first, default 0)
    #[serde(default)]
    pub priority: i32,
    /// Optional description
    #[serde(default)]
    pub description: Option<String>,
    /// The scope level this policy was defined at
    #[serde(default)]
    pub scope: PolicyScope,
}

/// The scope level at which a policy is defined.
#[cfg(feature = "enterprise")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PolicyScope {
    /// Global policies apply to all organizations
    Global = 0,
    /// Organization-level policies
    Organization = 1,
    /// Team-level policies
    Team = 2,
    /// User-level policies (most specific)
    User = 3,
}

#[cfg(feature = "enterprise")]
impl Default for PolicyScope {
    fn default() -> Self {
        Self::Global
    }
}

/// An organization in the tenant hierarchy.
#[cfg(feature = "enterprise")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Org {
    /// Unique organization identifier
    pub id: String,
    /// Organization display name
    pub name: String,
    /// Organization-level policies
    #[serde(default)]
    pub policies: Vec<Policy>,
    /// Teams within this organization
    #[serde(default)]
    pub teams: Vec<Team>,
}

/// A team within an organization.
#[cfg(feature = "enterprise")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Team {
    /// Unique team identifier
    pub id: String,
    /// Team display name
    pub name: String,
    /// Parent organization ID
    pub org_id: String,
    /// Team-level policies
    #[serde(default)]
    pub policies: Vec<Policy>,
    /// Members of this team (user IDs)
    #[serde(default)]
    pub members: Vec<String>,
}

/// Represents the full tenant hierarchy for policy resolution.
#[cfg(feature = "enterprise")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantHierarchy {
    /// Global policies that apply to all tenants
    #[serde(default)]
    pub global_policies: Vec<Policy>,
    /// Organizations in the hierarchy
    #[serde(default)]
    pub organizations: Vec<Org>,
}

#[cfg(feature = "enterprise")]
impl TenantHierarchy {
    /// Create a new empty tenant hierarchy.
    pub fn new() -> Self {
        Self {
            global_policies: Vec::new(),
            organizations: Vec::new(),
        }
    }

    /// Find an organization by ID.
    pub fn find_org(&self, org_id: &str) -> Option<&Org> {
        self.organizations.iter().find(|o| o.id == org_id)
    }

    /// Find a team by org and team ID.
    pub fn find_team(&self, org_id: &str, team_id: &str) -> Option<&Team> {
        self.find_org(org_id)
            .and_then(|org| org.teams.iter().find(|t| t.id == team_id))
    }

    /// Find which team a user belongs to within an organization.
    pub fn find_user_team(&self, org_id: &str, user_id: &str) -> Option<&Team> {
        self.find_org(org_id).and_then(|org| {
            org.teams
                .iter()
                .find(|t| t.members.iter().any(|m| m == user_id))
        })
    }
}

#[cfg(feature = "enterprise")]
impl Default for TenantHierarchy {
    fn default() -> Self {
        Self::new()
    }
}

/// Policy resolution order determining how policies are combined.
#[cfg(feature = "enterprise")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyResolutionOrder {
    /// Most specific scope wins (User > Team > Org > Global)
    /// This is the default. More specific policies override less specific ones.
    MostSpecificWins,
    /// Least specific scope wins (Global > Org > Team > User)
    /// Useful when org admins need absolute control.
    LeastSpecificWins,
}

/// Resolve effective policies by combining policies from all hierarchy levels.
///
/// Resolution rules:
/// 1. Policies are grouped by action
/// 2. For each action, policies are sorted by specificity
/// 3. The most specific policy wins (User > Team > Org > Global)
/// 4. CRITICAL: `forbid` ALWAYS overrides `permit` regardless of specificity
///
/// This means if a Global policy forbids an action, no User-level permit
/// can override it. This ensures security invariants are maintained.
#[cfg(feature = "enterprise")]
pub fn resolve_effective_policies(
    global_policies: &[Policy],
    org_policies: &[Policy],
    team_policies: &[Policy],
    user_policies: &[Policy],
) -> Vec<Policy> {
    resolve_with_order(
        global_policies,
        org_policies,
        team_policies,
        user_policies,
        PolicyResolutionOrder::MostSpecificWins,
    )
}

/// Resolve effective policies with a specified resolution order.
#[cfg(feature = "enterprise")]
pub fn resolve_with_order(
    global_policies: &[Policy],
    org_policies: &[Policy],
    team_policies: &[Policy],
    user_policies: &[Policy],
    _order: PolicyResolutionOrder,
) -> Vec<Policy> {
    use std::collections::HashMap;

    // Group all policies by action
    let mut by_action: HashMap<String, Vec<&Policy>> = HashMap::new();

    for policy in global_policies
        .iter()
        .chain(org_policies.iter())
        .chain(team_policies.iter())
        .chain(user_policies.iter())
    {
        by_action
            .entry(policy.action.clone())
            .or_default()
            .push(policy);
    }

    let mut effective = Vec::new();

    for (action, policies) in &by_action {
        // CRITICAL INVARIANT: forbid ALWAYS wins over permit
        // Check if ANY policy at ANY level forbids this action
        let has_forbid = policies
            .iter()
            .any(|p| p.decision == PolicyDecision::Forbid);

        if has_forbid {
            // Find the most specific forbid policy (highest scope + priority)
            let forbid_policy = policies
                .iter()
                .filter(|p| p.decision == PolicyDecision::Forbid)
                .max_by(|a, b| {
                    a.scope
                        .cmp(&b.scope)
                        .then_with(|| a.priority.cmp(&b.priority))
                })
                .unwrap(); // Safe: we know there's at least one forbid

            effective.push((*forbid_policy).clone());
        } else {
            // No forbids -- find the most specific permit
            let permit_policy = policies
                .iter()
                .filter(|p| p.decision == PolicyDecision::Permit)
                .max_by(|a, b| {
                    a.scope
                        .cmp(&b.scope)
                        .then_with(|| a.priority.cmp(&b.priority))
                });

            if let Some(policy) = permit_policy {
                effective.push((*policy).clone());
            } else {
                // No explicit policy -- create a default deny for this action
                effective.push(Policy {
                    id: format!("default-deny-{}", action),
                    name: format!("Default deny for {}", action),
                    action: action.clone(),
                    decision: PolicyDecision::Forbid,
                    priority: 0,
                    description: Some("No explicit policy found; default deny".to_string()),
                    scope: PolicyScope::Global,
                });
            }
        }
    }

    // Sort by action name for deterministic output
    effective.sort_by(|a, b| a.action.cmp(&b.action));
    effective
}

/// Check if a specific action is permitted given resolved policies.
#[cfg(feature = "enterprise")]
pub fn is_action_permitted(policies: &[Policy], action: &str) -> bool {
    policies
        .iter()
        .find(|p| p.action == action)
        .is_some_and(|p| p.decision == PolicyDecision::Permit)
}

#[cfg(all(test, feature = "enterprise"))]
mod tests {
    use super::*;

    fn make_policy(
        id: &str,
        action: &str,
        decision: PolicyDecision,
        scope: PolicyScope,
        priority: i32,
    ) -> Policy {
        Policy {
            id: id.to_string(),
            name: format!("{} policy", id),
            action: action.to_string(),
            decision,
            priority,
            description: None,
            scope,
        }
    }

    #[test]
    fn test_most_specific_wins_for_permits() {
        let global = vec![make_policy(
            "g1",
            "Run",
            PolicyDecision::Permit,
            PolicyScope::Global,
            0,
        )];
        let org = vec![make_policy(
            "o1",
            "Run",
            PolicyDecision::Permit,
            PolicyScope::Organization,
            0,
        )];
        let team = vec![];
        let user = vec![make_policy(
            "u1",
            "Run",
            PolicyDecision::Permit,
            PolicyScope::User,
            0,
        )];

        let effective = resolve_effective_policies(&global, &org, &team, &user);
        assert_eq!(effective.len(), 1);
        // User-level policy should win (most specific)
        assert_eq!(effective[0].id, "u1");
        assert_eq!(effective[0].decision, PolicyDecision::Permit);
    }

    #[test]
    fn test_forbid_always_overrides_permit() {
        // Global forbids, user permits -- forbid MUST win
        let global = vec![make_policy(
            "g1",
            "Network",
            PolicyDecision::Forbid,
            PolicyScope::Global,
            0,
        )];
        let org = vec![];
        let team = vec![];
        let user = vec![make_policy(
            "u1",
            "Network",
            PolicyDecision::Permit,
            PolicyScope::User,
            100,
        )];

        let effective = resolve_effective_policies(&global, &org, &team, &user);
        assert_eq!(effective.len(), 1);
        assert_eq!(effective[0].action, "Network");
        assert_eq!(effective[0].decision, PolicyDecision::Forbid);
    }

    #[test]
    fn test_forbid_at_any_level_wins() {
        // Even if team permits, org forbid wins
        let global = vec![];
        let org = vec![make_policy(
            "o1",
            "Mount",
            PolicyDecision::Forbid,
            PolicyScope::Organization,
            0,
        )];
        let team = vec![make_policy(
            "t1",
            "Mount",
            PolicyDecision::Permit,
            PolicyScope::Team,
            10,
        )];
        let user = vec![make_policy(
            "u1",
            "Mount",
            PolicyDecision::Permit,
            PolicyScope::User,
            20,
        )];

        let effective = resolve_effective_policies(&global, &org, &team, &user);
        assert_eq!(effective.len(), 1);
        assert_eq!(effective[0].decision, PolicyDecision::Forbid);
    }

    #[test]
    fn test_multiple_actions_resolved_independently() {
        let global = vec![
            make_policy("g1", "Run", PolicyDecision::Permit, PolicyScope::Global, 0),
            make_policy(
                "g2",
                "Network",
                PolicyDecision::Forbid,
                PolicyScope::Global,
                0,
            ),
        ];
        let org = vec![];
        let team = vec![];
        let user = vec![make_policy(
            "u1",
            "Network",
            PolicyDecision::Permit,
            PolicyScope::User,
            0,
        )];

        let effective = resolve_effective_policies(&global, &org, &team, &user);
        assert_eq!(effective.len(), 2);

        // Network should be forbidden (global forbid overrides user permit)
        assert!(!is_action_permitted(&effective, "Network"));
        // Run should be permitted
        assert!(is_action_permitted(&effective, "Run"));
    }

    #[test]
    fn test_default_deny_when_no_policy() {
        // If no policy exists for an action, it should not be permitted
        let effective = resolve_effective_policies(&[], &[], &[], &[]);
        assert!(effective.is_empty());

        // Actions with no matching policy are not permitted
        assert!(!is_action_permitted(&effective, "Run"));
    }

    #[test]
    fn test_priority_within_same_scope() {
        let org = vec![
            make_policy(
                "o1",
                "Run",
                PolicyDecision::Permit,
                PolicyScope::Organization,
                10,
            ),
            make_policy(
                "o2",
                "Run",
                PolicyDecision::Permit,
                PolicyScope::Organization,
                20,
            ),
        ];

        let effective = resolve_effective_policies(&[], &org, &[], &[]);
        assert_eq!(effective.len(), 1);
        // Higher priority should win
        assert_eq!(effective[0].id, "o2");
    }

    #[test]
    fn test_tenant_hierarchy_find_org() {
        let hierarchy = TenantHierarchy {
            global_policies: vec![],
            organizations: vec![
                Org {
                    id: "acme".to_string(),
                    name: "Acme Corp".to_string(),
                    policies: vec![],
                    teams: vec![],
                },
                Org {
                    id: "globex".to_string(),
                    name: "Globex Inc".to_string(),
                    policies: vec![],
                    teams: vec![],
                },
            ],
        };

        assert!(hierarchy.find_org("acme").is_some());
        assert_eq!(hierarchy.find_org("acme").unwrap().name, "Acme Corp");
        assert!(hierarchy.find_org("unknown").is_none());
    }

    #[test]
    fn test_tenant_hierarchy_find_team() {
        let hierarchy = TenantHierarchy {
            global_policies: vec![],
            organizations: vec![Org {
                id: "acme".to_string(),
                name: "Acme Corp".to_string(),
                policies: vec![],
                teams: vec![
                    Team {
                        id: "platform".to_string(),
                        name: "Platform Team".to_string(),
                        org_id: "acme".to_string(),
                        policies: vec![],
                        members: vec!["user-1".to_string(), "user-2".to_string()],
                    },
                    Team {
                        id: "ml".to_string(),
                        name: "ML Research".to_string(),
                        org_id: "acme".to_string(),
                        policies: vec![],
                        members: vec!["user-3".to_string()],
                    },
                ],
            }],
        };

        assert!(hierarchy.find_team("acme", "platform").is_some());
        assert_eq!(
            hierarchy.find_team("acme", "platform").unwrap().name,
            "Platform Team"
        );
        assert!(hierarchy.find_team("acme", "unknown").is_none());
        assert!(hierarchy.find_team("unknown", "platform").is_none());
    }

    #[test]
    fn test_tenant_hierarchy_find_user_team() {
        let hierarchy = TenantHierarchy {
            global_policies: vec![],
            organizations: vec![Org {
                id: "acme".to_string(),
                name: "Acme Corp".to_string(),
                policies: vec![],
                teams: vec![
                    Team {
                        id: "platform".to_string(),
                        name: "Platform".to_string(),
                        org_id: "acme".to_string(),
                        policies: vec![],
                        members: vec!["alice".to_string(), "bob".to_string()],
                    },
                    Team {
                        id: "ml".to_string(),
                        name: "ML".to_string(),
                        org_id: "acme".to_string(),
                        policies: vec![],
                        members: vec!["carol".to_string()],
                    },
                ],
            }],
        };

        let team = hierarchy.find_user_team("acme", "alice");
        assert!(team.is_some());
        assert_eq!(team.unwrap().id, "platform");

        let team = hierarchy.find_user_team("acme", "carol");
        assert!(team.is_some());
        assert_eq!(team.unwrap().id, "ml");

        assert!(hierarchy.find_user_team("acme", "unknown").is_none());
    }

    #[test]
    fn test_policy_scope_ordering() {
        assert!(PolicyScope::Global < PolicyScope::Organization);
        assert!(PolicyScope::Organization < PolicyScope::Team);
        assert!(PolicyScope::Team < PolicyScope::User);
    }

    #[test]
    fn test_is_action_permitted() {
        let policies = vec![
            make_policy("p1", "Run", PolicyDecision::Permit, PolicyScope::User, 0),
            make_policy(
                "p2",
                "Network",
                PolicyDecision::Forbid,
                PolicyScope::Global,
                0,
            ),
        ];

        assert!(is_action_permitted(&policies, "Run"));
        assert!(!is_action_permitted(&policies, "Network"));
        assert!(!is_action_permitted(&policies, "Unknown"));
    }

    #[test]
    fn test_org_serialization() {
        let org = Org {
            id: "acme".to_string(),
            name: "Acme Corp".to_string(),
            policies: vec![make_policy(
                "p1",
                "Run",
                PolicyDecision::Permit,
                PolicyScope::Organization,
                0,
            )],
            teams: vec![],
        };

        let json = serde_json::to_string(&org).unwrap();
        let deserialized: Org = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, "acme");
        assert_eq!(deserialized.policies.len(), 1);
    }
}
