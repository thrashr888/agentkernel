//! Integration tests for the enterprise policy engine components.
//!
//! Tests cover:
//! - Ed25519 sign + verify round-trip
//! - Cedar policy evaluation (permit/deny scenarios)
//! - Policy cache store/load/expiry
//! - JWT validation with mock JWKS
//! - OIDC device flow response parsing
//! - Multi-tenant policy resolution (forbid overrides permit)
//! - Offline modes (fail_closed, cached_with_expiry)
//! - Audit log streaming
//!
//! Run with: cargo test --test enterprise_policy_test --features enterprise
#![cfg(feature = "enterprise")]

use agentkernel::policy::cedar::{Action, CedarEngine, PolicyEffect, Principal, Resource};
use agentkernel::policy::cache::{OfflineMode, PolicyCache};
use agentkernel::policy::signing::{PolicyBundle, TrustAnchor, sign_bundle, verify_bundle};
use agentkernel::policy::tenant::{
    Org, Policy, PolicyDecision as TenantPolicyDecision, PolicyScope, Team, TenantHierarchy,
    is_action_permitted, resolve_effective_policies,
};
use agentkernel::policy::streaming::{
    AuditEvent as StreamAuditEvent, AuditStreamConfig, AuditStreamer, EventOutcome,
    StreamDestination, new_audit_event,
};
use agentkernel::identity::{
    AgentIdentity, JwtClaims, to_cedar_context, to_cedar_principal, validate_api_key,
};
use agentkernel::identity::oidc::{
    DeviceAuthResponse, OidcConfig, OidcDeviceFlow, StoredTokens, TokenResponse,
};

use chrono::Utc;
use ed25519_dalek::SigningKey;
use std::time::Duration;

// === Helper Functions ===

fn test_keypair() -> (SigningKey, Vec<u8>, String) {
    let signing_key = SigningKey::from_bytes(&[42u8; 32]);
    let verifying_key = signing_key.verifying_key();
    let public_bytes = verifying_key.to_bytes().to_vec();
    let key_id = "integration-test-key".to_string();
    (signing_key, public_bytes, key_id)
}

fn test_trust_anchor(public_key: Vec<u8>, key_id: &str) -> TrustAnchor {
    TrustAnchor {
        key_id: key_id.to_string(),
        public_key,
        valid_from: Utc::now() - chrono::Duration::hours(1),
        valid_until: Some(Utc::now() + chrono::Duration::hours(24)),
    }
}

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
        name: "integration-sandbox".to_string(),
        agent_type: "claude".to_string(),
        runtime: "python".to_string(),
    }
}

fn make_tenant_policy(
    id: &str,
    action: &str,
    decision: TenantPolicyDecision,
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

// === Ed25519 Sign + Verify Round-Trip ===

#[test]
fn test_ed25519_sign_verify_roundtrip() {
    let (signing_key, public_key, key_id) = test_keypair();
    let anchor = test_trust_anchor(public_key, &key_id);

    let policies = r#"
permit(
    principal is AgentKernel::User,
    action == AgentKernel::Action::"Run",
    resource is AgentKernel::Sandbox
);
    "#;

    // Sign a policy bundle
    let bundle = sign_bundle(
        policies,
        1,
        Some(Utc::now() + chrono::Duration::hours(12)),
        &signing_key,
        &key_id,
    )
    .expect("Signing should succeed");

    assert_eq!(bundle.version, 1);
    assert_eq!(bundle.signer_key_id, key_id);
    assert_eq!(bundle.signature.len(), 64);

    // Verify the signature
    verify_bundle(&bundle, &[anchor.clone()], None)
        .expect("Verification should succeed");

    // Verify with version monotonicity
    verify_bundle(&bundle, &[anchor.clone()], Some(1))
        .expect("Version 1 should pass min_version=1");

    verify_bundle(&bundle, &[anchor.clone()], Some(0))
        .expect("Version 1 should pass min_version=0");
}

#[test]
fn test_ed25519_tampered_bundle_rejected() {
    let (signing_key, public_key, key_id) = test_keypair();
    let anchor = test_trust_anchor(public_key, &key_id);

    let mut bundle = sign_bundle(
        "permit(principal, action, resource);",
        1,
        None,
        &signing_key,
        &key_id,
    )
    .unwrap();

    // Tamper with the policies
    bundle.policies = "forbid(principal, action, resource);".to_string();

    // Verification must fail
    let result = verify_bundle(&bundle, &[anchor], None);
    assert!(result.is_err(), "Tampered bundle should fail verification");
}

#[test]
fn test_ed25519_version_downgrade_rejected() {
    let (signing_key, public_key, key_id) = test_keypair();
    let anchor = test_trust_anchor(public_key, &key_id);

    let bundle = sign_bundle(
        "permit(principal, action, resource);",
        5,
        None,
        &signing_key,
        &key_id,
    )
    .unwrap();

    // Should reject if min_version is higher than bundle version
    let result = verify_bundle(&bundle, &[anchor], Some(10));
    assert!(result.is_err(), "Downgrade should be rejected");
    assert!(result.unwrap_err().to_string().contains("older"));
}

// === Cedar Policy Evaluation ===

#[test]
fn test_cedar_permit_all_users() {
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
fn test_cedar_deny_no_matching_policy() {
    let policies = r#"
permit(
    principal is AgentKernel::User,
    action == AgentKernel::Action::"Run",
    resource is AgentKernel::Sandbox
);
    "#;

    let engine = CedarEngine::new(policies).unwrap();
    // Exec has no matching permit policy
    let decision = engine.evaluate(&test_principal(), Action::Exec, &test_resource(), None);

    assert!(!decision.is_permit());
    assert_eq!(decision.decision, PolicyEffect::Deny);
}

#[test]
fn test_cedar_explicit_forbid_overrides_permit() {
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

    let engine = CedarEngine::new(policies).unwrap();

    // MFA user: permitted (forbid condition doesn't match)
    let mfa_user = test_principal();
    let decision = engine.evaluate(&mfa_user, Action::Network, &test_resource(), None);
    assert!(decision.is_permit());

    // Non-MFA user: denied (forbid condition matches)
    let mut no_mfa_user = test_principal();
    no_mfa_user.mfa_verified = false;
    let decision = engine.evaluate(&no_mfa_user, Action::Network, &test_resource(), None);
    assert!(!decision.is_permit());
}

#[test]
fn test_cedar_role_based_access() {
    let policies = r#"
permit(
    principal is AgentKernel::User,
    action == AgentKernel::Action::"Create",
    resource is AgentKernel::Sandbox
) when {
    principal.roles.contains("admin")
};
    "#;

    let engine = CedarEngine::new(policies).unwrap();

    // Developer (not admin): denied
    let decision = engine.evaluate(&test_principal(), Action::Create, &test_resource(), None);
    assert!(!decision.is_permit());

    // Admin: permitted
    let mut admin = test_principal();
    admin.roles = vec!["admin".to_string()];
    let decision = engine.evaluate(&admin, Action::Create, &test_resource(), None);
    assert!(decision.is_permit());
}

#[test]
fn test_cedar_empty_policy_denies_all() {
    let engine = CedarEngine::new("").unwrap();
    let decision = engine.evaluate(&test_principal(), Action::Run, &test_resource(), None);
    assert!(!decision.is_permit(), "Empty policy should deny by default");
}

// === Policy Cache Store/Load/Expiry ===

#[test]
fn test_cache_store_and_load() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cache = PolicyCache::new(
        tmp.path().join("policies"),
        OfflineMode::CachedIndefinite,
    );

    let bundle = PolicyBundle {
        policies: "permit(principal, action, resource);".to_string(),
        version: 42,
        expires_at: Some(Utc::now() + chrono::Duration::hours(24)),
        signature: vec![0u8; 64],
        signer_key_id: "test-key".to_string(),
    };

    // Store
    cache.store(&bundle).unwrap();

    // Load
    let loaded = cache.load().unwrap();
    assert!(loaded.is_some());
    let loaded = loaded.unwrap();
    assert_eq!(loaded.version, 42);
    assert_eq!(loaded.policies, bundle.policies);
}

#[test]
fn test_cache_empty_returns_none() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cache = PolicyCache::new(
        tmp.path().join("nonexistent"),
        OfflineMode::CachedIndefinite,
    );

    let loaded = cache.load().unwrap();
    assert!(loaded.is_none());
}

#[test]
fn test_cache_expiry_with_zero_ttl() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cache = PolicyCache::new(
        tmp.path().join("policies"),
        OfflineMode::CachedWithExpiry {
            max_age: Duration::from_secs(0),
        },
    );

    let bundle = PolicyBundle {
        policies: "permit(principal, action, resource);".to_string(),
        version: 1,
        expires_at: None,
        signature: vec![0u8; 64],
        signer_key_id: "test".to_string(),
    };

    cache.store(&bundle).unwrap();

    // Should fail because cache is immediately expired
    let result = cache.load();
    assert!(result.is_err(), "Expired cache should error");
}

#[test]
fn test_cache_indefinite_never_expires() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cache = PolicyCache::new(
        tmp.path().join("policies"),
        OfflineMode::CachedIndefinite,
    );

    let bundle = PolicyBundle {
        policies: "permit(principal, action, resource);".to_string(),
        version: 1,
        expires_at: None,
        signature: vec![0u8; 64],
        signer_key_id: "test".to_string(),
    };

    cache.store(&bundle).unwrap();
    let loaded = cache.load().unwrap();
    assert!(loaded.is_some(), "CachedIndefinite should never expire");
}

// === JWT Validation (unit-level, mock JWKS requires external service) ===

#[test]
fn test_agent_identity_from_jwt_claims() {
    let claims = JwtClaims {
        sub: "user-123".to_string(),
        email: "alice@acme.com".to_string(),
        org_id: "acme-corp".to_string(),
        roles: vec!["developer".to_string(), "admin".to_string()],
        mfa_verified: true,
        exp: Some(9999999999),
        iat: Some(1000000000),
    };

    let identity = AgentIdentity::from_jwt(claims);
    assert!(identity.is_authenticated());
    assert_eq!(identity.subject(), Some("user-123"));
    assert_eq!(identity.email(), Some("alice@acme.com"));
    assert_eq!(identity.org_id(), Some("acme-corp"));
    assert!(identity.has_role("developer"));
    assert!(identity.has_role("admin"));
    assert!(!identity.has_role("viewer"));
    assert!(identity.mfa_verified());
}

#[test]
fn test_api_key_validation() {
    let result = validate_api_key("ak_correct_key_12345", "ak_correct_key_12345");
    assert!(result.is_ok());
    assert!(result.unwrap().is_authenticated());

    let result = validate_api_key("wrong_key", "ak_correct_key_12345");
    assert!(result.is_err());
}

#[test]
fn test_cedar_principal_from_identity() {
    // JWT identity
    let claims = JwtClaims {
        sub: "user-abc".to_string(),
        email: "test@test.com".to_string(),
        org_id: "org-1".to_string(),
        roles: vec![],
        mfa_verified: false,
        exp: None,
        iat: None,
    };
    let jwt_identity = AgentIdentity::from_jwt(claims);
    let principal = to_cedar_principal(&jwt_identity);
    assert_eq!(principal, r#"AgentKernel::User::"user-abc""#);

    // API key identity
    let api_identity = AgentIdentity::from_api_key("ak_test_12345678rest".to_string());
    let principal = to_cedar_principal(&api_identity);
    assert_eq!(principal, r#"AgentKernel::ApiClient::"ak_test_""#);

    // Anonymous
    let anon = AgentIdentity::anonymous();
    let principal = to_cedar_principal(&anon);
    assert!(principal.contains("Anonymous"));
}

#[test]
fn test_cedar_context_from_identity() {
    let claims = JwtClaims {
        sub: "user-1".to_string(),
        email: "user@org.com".to_string(),
        org_id: "org-1".to_string(),
        roles: vec!["dev".to_string()],
        mfa_verified: true,
        exp: None,
        iat: None,
    };
    let identity = AgentIdentity::from_jwt(claims);
    let context = to_cedar_context(&identity);

    assert_eq!(context.get("email").unwrap(), "user@org.com");
    assert_eq!(context.get("org_id").unwrap(), "org-1");
    assert_eq!(context.get("mfa_verified").unwrap(), true);
    assert_eq!(context.get("is_authenticated").unwrap(), true);
}

#[test]
#[ignore] // Requires external JWKS endpoint
fn test_jwt_validation_with_real_jwks() {
    // This test would validate a real JWT against a real JWKS endpoint.
    // Ignored by default because it requires network access.
    // To run: cargo test --test enterprise_policy_test --features enterprise -- --ignored
}

// === OIDC Device Flow Response Parsing ===

#[test]
fn test_oidc_device_auth_response_parsing() {
    let json = r#"{
        "device_code": "Gi1xGQJ_PmNPVR-some-device-code",
        "user_code": "WDJB-MJHT",
        "verification_uri": "https://login.example.com/activate",
        "verification_uri_complete": "https://login.example.com/activate?user_code=WDJB-MJHT",
        "expires_in": 900,
        "interval": 5
    }"#;

    let response: DeviceAuthResponse = serde_json::from_str(json).unwrap();
    assert_eq!(response.user_code, "WDJB-MJHT");
    assert_eq!(response.expires_in, 900);
    assert_eq!(response.interval, 5);
    assert!(response.verification_uri_complete.is_some());
}

#[test]
fn test_oidc_token_response_parsing() {
    let json = r#"{
        "access_token": "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.test",
        "id_token": "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.claims",
        "refresh_token": "v1.refresh-token-value",
        "token_type": "Bearer",
        "expires_in": 3600,
        "scope": "openid profile email"
    }"#;

    let token: TokenResponse = serde_json::from_str(json).unwrap();
    assert_eq!(token.token_type, "Bearer");
    assert_eq!(token.expires_in, Some(3600));
    assert!(token.id_token.is_some());
    assert!(token.refresh_token.is_some());
}

#[test]
fn test_oidc_config_discovery_parsing() {
    let json = r#"{
        "issuer": "https://accounts.example.com",
        "authorization_endpoint": "https://accounts.example.com/authorize",
        "token_endpoint": "https://accounts.example.com/token",
        "device_authorization_endpoint": "https://accounts.example.com/device/code",
        "jwks_uri": "https://accounts.example.com/.well-known/jwks.json",
        "userinfo_endpoint": "https://accounts.example.com/userinfo",
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "urn:ietf:params:oauth:grant-type:device_code"]
    }"#;

    let config: OidcConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.issuer, "https://accounts.example.com");
    assert!(config.device_authorization_endpoint.is_some());
    assert_eq!(
        config.jwks_uri,
        "https://accounts.example.com/.well-known/jwks.json"
    );
}

#[test]
fn test_oidc_stored_tokens_roundtrip() {
    let stored = StoredTokens {
        access_token: "access-tok".to_string(),
        id_token: Some("id-tok".to_string()),
        refresh_token: Some("refresh-tok".to_string()),
        expires_at: Some("2026-12-31T23:59:59+00:00".to_string()),
        issuer: "https://example.com".to_string(),
        client_id: "my-client".to_string(),
    };

    let json = serde_json::to_string(&stored).unwrap();
    let restored: StoredTokens = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.access_token, "access-tok");
    assert_eq!(restored.client_id, "my-client");
    assert!(restored.refresh_token.is_some());
}

#[test]
fn test_oidc_device_flow_construction() {
    let flow = OidcDeviceFlow::new(
        "https://accounts.example.com".to_string(),
        "test-client-id".to_string(),
    );
    assert_eq!(flow.discovery_url, "https://accounts.example.com");
    assert_eq!(flow.client_id, "test-client-id");
    assert!(flow.scopes.contains(&"openid".to_string()));
}

#[test]
#[ignore] // Requires external OIDC provider
fn test_oidc_device_flow_end_to_end() {
    // Requires a real OIDC provider with device flow support
}

// === Multi-Tenant Policy Resolution ===

#[test]
fn test_tenant_most_specific_wins_for_permits() {
    let global = vec![make_tenant_policy(
        "g1",
        "Run",
        TenantPolicyDecision::Permit,
        PolicyScope::Global,
        0,
    )];
    let org = vec![make_tenant_policy(
        "o1",
        "Run",
        TenantPolicyDecision::Permit,
        PolicyScope::Organization,
        0,
    )];
    let team = vec![];
    let user = vec![make_tenant_policy(
        "u1",
        "Run",
        TenantPolicyDecision::Permit,
        PolicyScope::User,
        0,
    )];

    let effective = resolve_effective_policies(&global, &org, &team, &user);
    assert_eq!(effective.len(), 1);
    assert_eq!(effective[0].id, "u1");
}

#[test]
fn test_tenant_forbid_overrides_permit_regardless_of_level() {
    // Global forbids Network; user permits Network
    // Forbid MUST win (security invariant)
    let global = vec![make_tenant_policy(
        "g-forbid",
        "Network",
        TenantPolicyDecision::Forbid,
        PolicyScope::Global,
        0,
    )];
    let user = vec![make_tenant_policy(
        "u-permit",
        "Network",
        TenantPolicyDecision::Permit,
        PolicyScope::User,
        100, // Even with high priority
    )];

    let effective = resolve_effective_policies(&global, &[], &[], &user);
    assert_eq!(effective.len(), 1);
    assert_eq!(effective[0].decision, TenantPolicyDecision::Forbid);
    assert!(!is_action_permitted(&effective, "Network"));
}

#[test]
fn test_tenant_org_forbid_overrides_team_and_user_permits() {
    let org = vec![make_tenant_policy(
        "org-forbid",
        "Mount",
        TenantPolicyDecision::Forbid,
        PolicyScope::Organization,
        0,
    )];
    let team = vec![make_tenant_policy(
        "team-permit",
        "Mount",
        TenantPolicyDecision::Permit,
        PolicyScope::Team,
        10,
    )];
    let user = vec![make_tenant_policy(
        "user-permit",
        "Mount",
        TenantPolicyDecision::Permit,
        PolicyScope::User,
        20,
    )];

    let effective = resolve_effective_policies(&[], &org, &team, &user);
    assert_eq!(effective.len(), 1);
    assert_eq!(effective[0].decision, TenantPolicyDecision::Forbid);
}

#[test]
fn test_tenant_multiple_actions_resolved_independently() {
    let global = vec![
        make_tenant_policy(
            "g-run",
            "Run",
            TenantPolicyDecision::Permit,
            PolicyScope::Global,
            0,
        ),
        make_tenant_policy(
            "g-net",
            "Network",
            TenantPolicyDecision::Forbid,
            PolicyScope::Global,
            0,
        ),
    ];

    let effective = resolve_effective_policies(&global, &[], &[], &[]);
    assert_eq!(effective.len(), 2);
    assert!(is_action_permitted(&effective, "Run"));
    assert!(!is_action_permitted(&effective, "Network"));
}

#[test]
fn test_tenant_hierarchy_lookup() {
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
                    name: "ML Research".to_string(),
                    org_id: "acme".to_string(),
                    policies: vec![],
                    members: vec!["carol".to_string()],
                },
            ],
        }],
    };

    assert!(hierarchy.find_org("acme").is_some());
    assert!(hierarchy.find_org("globex").is_none());
    assert!(hierarchy.find_team("acme", "platform").is_some());

    let alice_team = hierarchy.find_user_team("acme", "alice");
    assert!(alice_team.is_some());
    assert_eq!(alice_team.unwrap().id, "platform");

    let carol_team = hierarchy.find_user_team("acme", "carol");
    assert!(carol_team.is_some());
    assert_eq!(carol_team.unwrap().id, "ml");

    assert!(hierarchy.find_user_team("acme", "unknown").is_none());
}

// === Offline Modes ===

#[test]
fn test_offline_mode_fail_closed() {
    let mode = OfflineMode::from_config("fail_closed", 24);
    assert_eq!(mode, OfflineMode::FailClosed);
}

#[test]
fn test_offline_mode_cached_with_expiry() {
    let mode = OfflineMode::from_config("cached_with_expiry", 48);
    assert_eq!(
        mode,
        OfflineMode::CachedWithExpiry {
            max_age: Duration::from_secs(48 * 3600),
        }
    );
}

#[test]
fn test_offline_mode_cached_indefinite() {
    let mode = OfflineMode::from_config("cached_indefinite", 24);
    assert_eq!(mode, OfflineMode::CachedIndefinite);
}

#[test]
fn test_offline_mode_default_policy() {
    let mode = OfflineMode::from_config("default_policy", 24);
    assert_eq!(mode, OfflineMode::DefaultPolicy);
}

#[test]
fn test_offline_mode_unknown_falls_back() {
    let mode = OfflineMode::from_config("invalid_string", 12);
    assert_eq!(
        mode,
        OfflineMode::CachedWithExpiry {
            max_age: Duration::from_secs(12 * 3600),
        }
    );
}

// === Audit Log Streaming ===

#[tokio::test]
async fn test_stream_to_file() {
    let tmp = tempfile::TempDir::new().unwrap();
    let file_path = tmp.path().join("stream-audit.jsonl");

    let config = AuditStreamConfig {
        destination: StreamDestination::File {
            path: file_path.to_string_lossy().to_string(),
        },
        batch_size: 10,
        flush_interval_secs: 300,
        max_retries: 1,
        ocsf_enabled: true,
    };

    let streamer = AuditStreamer::new(config);

    let events = vec![
        new_audit_event("policy_decision", "Run", EventOutcome::Permit),
        new_audit_event("policy_decision", "Network", EventOutcome::Deny),
        new_audit_event("sandbox_operation", "Create", EventOutcome::Info),
    ];

    streamer.stream_events(events).await.unwrap();

    let content = std::fs::read_to_string(&file_path).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 3);

    // Verify each line is valid JSON with expected fields
    for line in &lines {
        let event: StreamAuditEvent = serde_json::from_str(line).unwrap();
        assert!(!event.uid.is_empty());
        assert_eq!(event.metadata.product_name, "agentkernel");
    }
}

#[tokio::test]
async fn test_stream_batching() {
    let tmp = tempfile::TempDir::new().unwrap();
    let file_path = tmp.path().join("batch-test.jsonl");

    let config = AuditStreamConfig {
        destination: StreamDestination::File {
            path: file_path.to_string_lossy().to_string(),
        },
        batch_size: 3,
        flush_interval_secs: 300,
        max_retries: 1,
        ocsf_enabled: true,
    };

    let streamer = AuditStreamer::new(config);

    // Queue 2 events (below batch threshold)
    streamer
        .queue_event(new_audit_event("test", "action1", EventOutcome::Permit))
        .await
        .unwrap();
    streamer
        .queue_event(new_audit_event("test", "action2", EventOutcome::Permit))
        .await
        .unwrap();
    assert_eq!(streamer.buffered_count().await, 2);

    // Third event triggers batch flush
    streamer
        .queue_event(new_audit_event("test", "action3", EventOutcome::Permit))
        .await
        .unwrap();
    assert_eq!(streamer.buffered_count().await, 0);

    let content = std::fs::read_to_string(&file_path).unwrap();
    assert_eq!(content.lines().count(), 3);
}

#[tokio::test]
async fn test_stream_manual_flush() {
    let tmp = tempfile::TempDir::new().unwrap();
    let file_path = tmp.path().join("flush-test.jsonl");

    let config = AuditStreamConfig {
        destination: StreamDestination::File {
            path: file_path.to_string_lossy().to_string(),
        },
        batch_size: 100, // High threshold
        flush_interval_secs: 300,
        max_retries: 1,
        ocsf_enabled: true,
    };

    let streamer = AuditStreamer::new(config);

    streamer
        .queue_event(new_audit_event("test", "action", EventOutcome::Permit))
        .await
        .unwrap();
    assert!(!file_path.exists(), "File should not exist before flush");

    streamer.flush().await.unwrap();
    assert_eq!(streamer.buffered_count().await, 0);

    let content = std::fs::read_to_string(&file_path).unwrap();
    assert_eq!(content.lines().count(), 1);
}

#[test]
fn test_stream_config_deserialization() {
    let json = r#"{
        "destination": {"type": "http_webhook", "url": "https://hooks.example.com/audit", "authorization": "Bearer tok"},
        "batch_size": 50,
        "flush_interval_secs": 60,
        "max_retries": 5,
        "ocsf_enabled": true
    }"#;

    let config: AuditStreamConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.batch_size, 50);
    assert_eq!(config.max_retries, 5);
}

// === Cross-Component Integration ===

#[test]
fn test_signed_bundle_used_in_cedar_engine() {
    let (signing_key, public_key, key_id) = test_keypair();
    let anchor = test_trust_anchor(public_key, &key_id);

    let policies = r#"
permit(
    principal is AgentKernel::User,
    action == AgentKernel::Action::"Run",
    resource is AgentKernel::Sandbox
) when {
    principal.roles.contains("developer")
};
    "#;

    // Sign the policies
    let bundle = sign_bundle(
        policies,
        1,
        Some(Utc::now() + chrono::Duration::hours(1)),
        &signing_key,
        &key_id,
    )
    .unwrap();

    // Verify the signature
    verify_bundle(&bundle, &[anchor], None).unwrap();

    // Use the policies in Cedar engine
    let engine = CedarEngine::new(&bundle.policies).unwrap();

    let developer = test_principal();
    let decision = engine.evaluate(&developer, Action::Run, &test_resource(), None);
    assert!(decision.is_permit());

    let mut viewer = test_principal();
    viewer.roles = vec!["viewer".to_string()];
    let decision = engine.evaluate(&viewer, Action::Run, &test_resource(), None);
    assert!(!decision.is_permit());
}

#[test]
fn test_cache_roundtrip_preserves_bundle_for_cedar() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cache = PolicyCache::new(
        tmp.path().join("policies"),
        OfflineMode::CachedIndefinite,
    );

    let policies = r#"
permit(
    principal is AgentKernel::User,
    action == AgentKernel::Action::"Exec",
    resource is AgentKernel::Sandbox
);
    "#;

    let bundle = PolicyBundle {
        policies: policies.to_string(),
        version: 10,
        expires_at: None,
        signature: vec![0u8; 64],
        signer_key_id: "test".to_string(),
    };

    cache.store(&bundle).unwrap();
    let loaded = cache.load().unwrap().unwrap();

    // Loaded policies should work in Cedar
    let engine = CedarEngine::new(&loaded.policies).unwrap();
    let decision = engine.evaluate(&test_principal(), Action::Exec, &test_resource(), None);
    assert!(decision.is_permit());
}

#[test]
fn test_identity_to_tenant_resolution() {
    // Create identity from JWT
    let claims = JwtClaims {
        sub: "alice".to_string(),
        email: "alice@acme.com".to_string(),
        org_id: "acme".to_string(),
        roles: vec!["developer".to_string()],
        mfa_verified: true,
        exp: None,
        iat: None,
    };
    let identity = AgentIdentity::from_jwt(claims);

    // Create tenant hierarchy
    let hierarchy = TenantHierarchy {
        global_policies: vec![make_tenant_policy(
            "global-run",
            "Run",
            TenantPolicyDecision::Permit,
            PolicyScope::Global,
            0,
        )],
        organizations: vec![Org {
            id: "acme".to_string(),
            name: "Acme".to_string(),
            policies: vec![make_tenant_policy(
                "org-net-forbid",
                "Network",
                TenantPolicyDecision::Forbid,
                PolicyScope::Organization,
                0,
            )],
            teams: vec![Team {
                id: "platform".to_string(),
                name: "Platform".to_string(),
                org_id: "acme".to_string(),
                policies: vec![make_tenant_policy(
                    "team-net-permit",
                    "Network",
                    TenantPolicyDecision::Permit,
                    PolicyScope::Team,
                    0,
                )],
                members: vec!["alice".to_string()],
            }],
        }],
    };

    // Look up identity's org and team
    let org_id = identity.org_id().unwrap();
    let user_id = identity.subject().unwrap();
    let org = hierarchy.find_org(org_id).unwrap();
    let team = hierarchy.find_user_team(org_id, user_id).unwrap();

    // Resolve policies: org forbids Network, team permits Network
    // Forbid MUST win
    let effective = resolve_effective_policies(
        &hierarchy.global_policies,
        &org.policies,
        &team.policies,
        &[], // No user-level policies
    );

    assert!(is_action_permitted(&effective, "Run"));
    assert!(!is_action_permitted(&effective, "Network"), "Org forbid must override team permit");
}
