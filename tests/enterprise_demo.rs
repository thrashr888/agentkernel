//! Enterprise policy demo
//!
//! Loads example Cedar policies, evaluates real scenarios,
//! and writes an audit log. Run with:
//!
//!   cargo test --features enterprise --test enterprise_demo -- --nocapture --ignored

#![cfg(feature = "enterprise")]

use agentkernel::policy::audit::{PolicyAuditLogger, PolicyDecisionLog};
use agentkernel::policy::cedar::{Action, CedarEngine, PolicyEffect, Principal, Resource};
use agentkernel::policy::signing::{TrustAnchor, sign_bundle, verify_bundle};
use chrono::Utc;
use ed25519_dalek::SigningKey;
use std::path::Path;
use tempfile::TempDir;

fn load_policy(filename: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples/enterprise/policies")
        .join(filename);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("Failed to read {}: {}", filename, e))
}

fn demo_principal(name: &str, email: &str, org: &str, roles: &[&str], mfa: bool) -> Principal {
    Principal {
        id: name.to_string(),
        email: email.to_string(),
        org_id: org.to_string(),
        roles: roles.iter().map(|r| r.to_string()).collect(),
        mfa_verified: mfa,
    }
}

fn demo_sandbox(name: &str, agent: &str, runtime: &str) -> Resource {
    Resource {
        name: name.to_string(),
        agent_type: agent.to_string(),
        runtime: runtime.to_string(),
    }
}

fn eval_and_log(
    engine: &CedarEngine,
    logger: &PolicyAuditLogger,
    principal: &Principal,
    action: Action,
    resource: &Resource,
    label: &str,
) {
    let decision = engine.evaluate(principal, action, resource, None);
    let symbol = if decision.is_permit() {
        "PERMIT"
    } else {
        "DENY  "
    };
    println!(
        "  {} | {:7} {} on {:20} | {}",
        symbol,
        format!("{}", action),
        principal.id,
        resource.name,
        label,
    );

    let effect = if decision.is_permit() {
        PolicyEffect::Permit
    } else {
        PolicyEffect::Deny
    };

    let entry = PolicyDecisionLog::new(
        &principal.id,
        action,
        &resource.name,
        effect,
        decision.matched_policies.clone(),
        decision.evaluation_time_us,
        Some(principal.org_id.clone()),
        Some(decision.reason.clone()),
    );
    let _ = logger.log_decision(&entry);
}

#[test]
#[ignore]
fn demo_default_policy() {
    println!("\n=== Default Policy ===");
    println!("  Permits all authenticated users for basic operations.\n");

    let policies = load_policy("default.cedar");
    let engine = CedarEngine::new(&policies).unwrap();
    let tmp = TempDir::new().unwrap();
    let logger = PolicyAuditLogger::new(tmp.path().join("audit.jsonl"));

    let alice = demo_principal("alice", "alice@acme.com", "acme-corp", &["developer"], true);
    let sandbox = demo_sandbox("my-project", "claude", "python");

    eval_and_log(
        &engine,
        &logger,
        &alice,
        Action::Create,
        &sandbox,
        "authenticated user",
    );
    eval_and_log(
        &engine,
        &logger,
        &alice,
        Action::Run,
        &sandbox,
        "authenticated user",
    );
    eval_and_log(
        &engine,
        &logger,
        &alice,
        Action::Exec,
        &sandbox,
        "authenticated user",
    );
    eval_and_log(
        &engine,
        &logger,
        &alice,
        Action::Attach,
        &sandbox,
        "authenticated user",
    );
    eval_and_log(
        &engine,
        &logger,
        &alice,
        Action::Mount,
        &sandbox,
        "not in default policy",
    );
    eval_and_log(
        &engine,
        &logger,
        &alice,
        Action::Network,
        &sandbox,
        "not in default policy",
    );

    println!("\n  Audit log (JSONL):");
    for entry in logger.read_all().unwrap() {
        println!("    {}", serde_json::to_string(&entry).unwrap());
    }
}

#[test]
#[ignore]
fn demo_rbac_policy() {
    println!("\n=== RBAC Policy ===");
    println!("  Role-based: developer, admin, viewer.\n");

    let policies = load_policy("rbac.cedar");
    let engine = CedarEngine::new(&policies).unwrap();
    let tmp = TempDir::new().unwrap();
    let logger = PolicyAuditLogger::new(tmp.path().join("audit.jsonl"));

    let dev = demo_principal("bob", "bob@acme.com", "acme-corp", &["developer"], true);
    let admin = demo_principal("carol", "carol@acme.com", "acme-corp", &["admin"], true);
    let viewer = demo_principal("dave", "dave@acme.com", "acme-corp", &["viewer"], false);
    let sandbox = demo_sandbox("api-server", "claude", "python");

    println!("  Developer (bob):");
    eval_and_log(
        &engine,
        &logger,
        &dev,
        Action::Create,
        &sandbox,
        "developer role",
    );
    eval_and_log(
        &engine,
        &logger,
        &dev,
        Action::Run,
        &sandbox,
        "developer role",
    );
    eval_and_log(
        &engine,
        &logger,
        &dev,
        Action::Mount,
        &sandbox,
        "developer can't mount",
    );
    eval_and_log(
        &engine,
        &logger,
        &dev,
        Action::Network,
        &sandbox,
        "developer can't network",
    );

    println!("\n  Admin (carol):");
    eval_and_log(
        &engine,
        &logger,
        &admin,
        Action::Create,
        &sandbox,
        "admin inherits all",
    );
    eval_and_log(
        &engine,
        &logger,
        &admin,
        Action::Mount,
        &sandbox,
        "admin can mount",
    );
    eval_and_log(
        &engine,
        &logger,
        &admin,
        Action::Network,
        &sandbox,
        "admin can network",
    );

    println!("\n  Viewer (dave):");
    eval_and_log(
        &engine,
        &logger,
        &viewer,
        Action::Attach,
        &sandbox,
        "viewer can attach",
    );
    eval_and_log(
        &engine,
        &logger,
        &viewer,
        Action::Create,
        &sandbox,
        "viewer can't create",
    );
    eval_and_log(
        &engine,
        &logger,
        &viewer,
        Action::Exec,
        &sandbox,
        "viewer can't exec",
    );

    println!("\n  Audit log (JSONL):");
    for entry in logger.read_all().unwrap() {
        println!("    {}", serde_json::to_string(&entry).unwrap());
    }
}

#[test]
#[ignore]
fn demo_mfa_policy() {
    println!("\n=== MFA Enforcement Policy ===");
    println!("  Network and Mount require MFA verification.\n");

    let policies = load_policy("mfa-required.cedar");
    let engine = CedarEngine::new(&policies).unwrap();
    let tmp = TempDir::new().unwrap();
    let logger = PolicyAuditLogger::new(tmp.path().join("audit.jsonl"));

    let mfa_user = demo_principal("alice", "alice@acme.com", "acme-corp", &["developer"], true);
    let no_mfa = demo_principal("eve", "eve@acme.com", "acme-corp", &["developer"], false);
    let sandbox = demo_sandbox("data-pipeline", "gemini", "python");

    println!("  With MFA (alice):");
    eval_and_log(
        &engine,
        &logger,
        &mfa_user,
        Action::Run,
        &sandbox,
        "basic op, no MFA needed",
    );
    eval_and_log(
        &engine,
        &logger,
        &mfa_user,
        Action::Network,
        &sandbox,
        "MFA verified -> permit",
    );
    eval_and_log(
        &engine,
        &logger,
        &mfa_user,
        Action::Mount,
        &sandbox,
        "MFA verified -> permit",
    );

    println!("\n  Without MFA (eve):");
    eval_and_log(
        &engine,
        &logger,
        &no_mfa,
        Action::Run,
        &sandbox,
        "basic op, no MFA needed",
    );
    eval_and_log(
        &engine,
        &logger,
        &no_mfa,
        Action::Network,
        &sandbox,
        "no MFA -> forbid",
    );
    eval_and_log(
        &engine,
        &logger,
        &no_mfa,
        Action::Mount,
        &sandbox,
        "no MFA -> forbid",
    );

    println!("\n  Audit log (JSONL):");
    for entry in logger.read_all().unwrap() {
        println!("    {}", serde_json::to_string(&entry).unwrap());
    }
}

#[test]
#[ignore]
fn demo_runtime_restrictions() {
    println!("\n=== Runtime Restriction Policy ===");
    println!("  Developers: Python/Node only. Platform team: Rust/Go. Codex agent blocked.\n");

    let policies = load_policy("runtime-restrictions.cedar");
    let engine = CedarEngine::new(&policies).unwrap();
    let tmp = TempDir::new().unwrap();
    let logger = PolicyAuditLogger::new(tmp.path().join("audit.jsonl"));

    let dev = demo_principal("alice", "alice@acme.com", "acme-corp", &["developer"], true);
    let platform = demo_principal("bob", "bob@acme.com", "acme-corp", &["platform"], true);

    let py_sandbox = demo_sandbox("ml-project", "claude", "python");
    let rust_sandbox = demo_sandbox("infra-tool", "claude", "rust");
    let codex_sandbox = demo_sandbox("codex-project", "codex", "python");

    println!("  Developer (alice):");
    eval_and_log(
        &engine,
        &logger,
        &dev,
        Action::Create,
        &py_sandbox,
        "python -> permit",
    );
    eval_and_log(
        &engine,
        &logger,
        &dev,
        Action::Create,
        &rust_sandbox,
        "rust -> deny",
    );

    println!("\n  Platform engineer (bob):");
    eval_and_log(
        &engine,
        &logger,
        &platform,
        Action::Create,
        &rust_sandbox,
        "rust -> permit",
    );
    eval_and_log(
        &engine,
        &logger,
        &platform,
        Action::Create,
        &py_sandbox,
        "python -> deny (no dev role)",
    );

    println!("\n  Codex agent (blocked for all):");
    eval_and_log(
        &engine,
        &logger,
        &dev,
        Action::Run,
        &codex_sandbox,
        "codex agent -> forbid",
    );
    eval_and_log(
        &engine,
        &logger,
        &platform,
        Action::Run,
        &codex_sandbox,
        "codex agent -> forbid",
    );

    println!("\n  Audit log (JSONL):");
    for entry in logger.read_all().unwrap() {
        println!("    {}", serde_json::to_string(&entry).unwrap());
    }
}

#[test]
#[ignore]
fn demo_signed_bundle() {
    println!("\n=== Signed Policy Bundle ===");
    println!("  Ed25519 signing, verification, and tamper detection.\n");

    // Generate a keypair
    let signing_key = SigningKey::from_bytes(&[42u8; 32]);
    let verifying_key = signing_key.verifying_key();
    let key_id = "prod-signing-key-2026";

    let anchor = TrustAnchor {
        key_id: key_id.to_string(),
        public_key: verifying_key.to_bytes().to_vec(),
        valid_from: Utc::now() - chrono::Duration::hours(1),
        valid_until: Some(Utc::now() + chrono::Duration::hours(8760)),
    };

    // Sign the RBAC policy
    let policies = load_policy("rbac.cedar");
    let bundle = sign_bundle(
        &policies,
        1,
        Some(Utc::now() + chrono::Duration::hours(24)),
        &signing_key,
        key_id,
    )
    .unwrap();

    println!("  Bundle version:   {}", bundle.version);
    println!("  Signer key:       {}", bundle.signer_key_id);
    println!("  Signature:        {} bytes", bundle.signature.len());
    println!("  Policy size:      {} bytes", bundle.policies.len());
    println!("  Expires:          {}", bundle.expires_at.unwrap());

    // Verify valid bundle
    print!("\n  Verify valid bundle:     ");
    match verify_bundle(&bundle, &[anchor.clone()], None) {
        Ok(()) => println!("PASS"),
        Err(e) => println!("FAIL: {}", e),
    }

    // Tamper and reverify
    let mut tampered = bundle.clone();
    tampered.policies = tampered.policies.replace("permit", "forbid");
    print!("  Verify tampered bundle:  ");
    match verify_bundle(&tampered, &[anchor.clone()], None) {
        Ok(()) => println!("FAIL (should have caught tamper!)"),
        Err(e) => println!("REJECTED: {}", e),
    }

    // Version monotonicity
    print!("  Verify downgrade (v1 < min v5): ");
    match verify_bundle(&bundle, &[anchor], Some(5)) {
        Ok(()) => println!("FAIL (should have caught downgrade!)"),
        Err(e) => println!("REJECTED: {}", e),
    }

    // Show the signed bundle JSON
    println!("\n  Signed bundle (JSON):");
    let json = serde_json::to_string_pretty(&bundle).unwrap();
    for line in json.lines() {
        println!("    {}", line);
    }
}
