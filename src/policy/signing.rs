//! Ed25519 cryptographic signing and verification for policy bundles.
//!
//! Ensures that policies fetched from a remote server have not been tampered
//! with. Each PolicyBundle carries an Ed25519 signature which is verified
//! against a set of trust anchors before the policies are loaded.

#![cfg(feature = "enterprise")]

use anyhow::{Context as _, Result, bail};
use chrono::{DateTime, Utc};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

/// A signed bundle of Cedar policies fetched from the policy server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyBundle {
    /// Raw Cedar policy text
    pub policies: String,
    /// Monotonically increasing version number
    pub version: u64,
    /// Optional expiry timestamp
    pub expires_at: Option<DateTime<Utc>>,
    /// Ed25519 signature over the canonical payload
    #[serde(with = "serde_bytes_base64")]
    pub signature: Vec<u8>,
    /// Key ID identifying which trust anchor signed this bundle
    pub signer_key_id: String,
}

/// A trust anchor holding a public key for signature verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustAnchor {
    /// Unique identifier for this key
    pub key_id: String,
    /// Ed25519 public key bytes (32 bytes)
    #[serde(with = "serde_bytes_base64")]
    pub public_key: Vec<u8>,
    /// When this key became valid
    pub valid_from: DateTime<Utc>,
    /// When this key expires (None = no expiry)
    pub valid_until: Option<DateTime<Utc>>,
}

impl TrustAnchor {
    /// Check if this trust anchor is currently valid.
    pub fn is_valid(&self) -> bool {
        let now = Utc::now();
        if now < self.valid_from {
            return false;
        }
        if let Some(until) = self.valid_until
            && now > until
        {
            return false;
        }
        true
    }
}

impl PolicyBundle {
    /// Compute the canonical payload that is signed.
    ///
    /// The payload is: `version || expires_at_rfc3339 || policies`
    /// This ensures version and expiry are covered by the signature.
    pub fn canonical_payload(&self) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&self.version.to_le_bytes());
        let expires_str = self.expires_at.map(|t| t.to_rfc3339()).unwrap_or_default();
        payload.extend_from_slice(expires_str.as_bytes());
        payload.extend_from_slice(self.policies.as_bytes());
        payload
    }
}

/// Verify a policy bundle's signature against a set of trust anchors.
///
/// Checks:
/// 1. The signer key ID matches a known trust anchor
/// 2. The trust anchor is currently valid
/// 3. The Ed25519 signature is valid over the canonical payload
/// 4. The bundle has not expired
/// 5. Version monotonicity (if `min_version` is provided)
pub fn verify_bundle(
    bundle: &PolicyBundle,
    trust_anchors: &[TrustAnchor],
    min_version: Option<u64>,
) -> Result<()> {
    // Find the matching trust anchor
    let anchor = trust_anchors
        .iter()
        .find(|a| a.key_id == bundle.signer_key_id)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No trust anchor found for signer key: {}",
                bundle.signer_key_id
            )
        })?;

    // Check trust anchor validity
    if !anchor.is_valid() {
        bail!("Trust anchor '{}' is not currently valid", anchor.key_id);
    }

    // Verify Ed25519 signature
    if anchor.public_key.len() != 32 {
        bail!(
            "Invalid public key length: expected 32, got {}",
            anchor.public_key.len()
        );
    }

    let verifying_key = VerifyingKey::from_bytes(
        anchor
            .public_key
            .as_slice()
            .try_into()
            .context("Public key must be 32 bytes")?,
    )
    .context("Invalid Ed25519 public key")?;

    if bundle.signature.len() != 64 {
        bail!(
            "Invalid signature length: expected 64, got {}",
            bundle.signature.len()
        );
    }

    let signature = Signature::from_bytes(
        bundle
            .signature
            .as_slice()
            .try_into()
            .context("Signature must be 64 bytes")?,
    );

    let payload = bundle.canonical_payload();
    verifying_key
        .verify(&payload, &signature)
        .context("Ed25519 signature verification failed")?;

    // Check expiry
    if let Some(expires_at) = bundle.expires_at
        && Utc::now() > expires_at
    {
        bail!("Policy bundle has expired (expired at {})", expires_at);
    }

    // Check version monotonicity
    if let Some(min_ver) = min_version
        && bundle.version < min_ver
    {
        bail!(
            "Policy bundle version {} is older than minimum required version {}",
            bundle.version,
            min_ver
        );
    }

    Ok(())
}

/// Sign a policy bundle with an Ed25519 private key.
///
/// Used for testing and tooling to create signed bundles.
pub fn sign_bundle(
    policies: &str,
    version: u64,
    expires_at: Option<DateTime<Utc>>,
    signing_key: &SigningKey,
    key_id: &str,
) -> Result<PolicyBundle> {
    let mut bundle = PolicyBundle {
        policies: policies.to_string(),
        version,
        expires_at,
        signature: vec![0u8; 64], // placeholder
        signer_key_id: key_id.to_string(),
    };

    let payload = bundle.canonical_payload();
    let signature = signing_key.sign(&payload);
    bundle.signature = signature.to_bytes().to_vec();

    Ok(bundle)
}

/// Helper module for base64 serialization of byte vectors.
mod serde_bytes_base64 {
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        serializer.serialize_str(&encoded)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        base64::engine::general_purpose::STANDARD
            .decode(&s)
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;

    fn test_keypair() -> (SigningKey, Vec<u8>, String) {
        let signing_key = SigningKey::from_bytes(&[1u8; 32]);
        let verifying_key = signing_key.verifying_key();
        let public_bytes = verifying_key.to_bytes().to_vec();
        let key_id = "test-key-1".to_string();
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

    #[test]
    fn test_sign_and_verify() {
        let (signing_key, public_key, key_id) = test_keypair();
        let anchor = test_trust_anchor(public_key, &key_id);

        let bundle = sign_bundle(
            "permit(principal, action, resource);",
            1,
            Some(Utc::now() + chrono::Duration::hours(1)),
            &signing_key,
            &key_id,
        )
        .unwrap();

        assert_eq!(bundle.version, 1);
        assert_eq!(bundle.signer_key_id, key_id);
        assert_eq!(bundle.signature.len(), 64);

        // Verification should succeed
        verify_bundle(&bundle, &[anchor], None).unwrap();
    }

    #[test]
    fn test_tampered_policies() {
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

        // Verification should fail
        let result = verify_bundle(&bundle, &[anchor], None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("signature"));
    }

    #[test]
    fn test_expired_bundle() {
        let (signing_key, public_key, key_id) = test_keypair();
        let anchor = test_trust_anchor(public_key, &key_id);

        let bundle = sign_bundle(
            "permit(principal, action, resource);",
            1,
            Some(Utc::now() - chrono::Duration::hours(1)), // Already expired
            &signing_key,
            &key_id,
        )
        .unwrap();

        let result = verify_bundle(&bundle, &[anchor], None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("expired"));
    }

    #[test]
    fn test_version_monotonicity() {
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

        // Should pass with min_version <= 5
        verify_bundle(&bundle, &[anchor.clone()], Some(5)).unwrap();
        verify_bundle(&bundle, &[anchor.clone()], Some(3)).unwrap();

        // Should fail with min_version > 5
        let result = verify_bundle(&bundle, &[anchor], Some(6));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("older"));
    }

    #[test]
    fn test_unknown_signer() {
        let (signing_key, _, key_id) = test_keypair();
        let bundle = sign_bundle(
            "permit(principal, action, resource);",
            1,
            None,
            &signing_key,
            &key_id,
        )
        .unwrap();

        // Use a different key_id for the anchor
        let other_anchor = TrustAnchor {
            key_id: "different-key".to_string(),
            public_key: vec![0u8; 32],
            valid_from: Utc::now() - chrono::Duration::hours(1),
            valid_until: None,
        };

        let result = verify_bundle(&bundle, &[other_anchor], None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No trust anchor"));
    }

    #[test]
    fn test_expired_trust_anchor() {
        let (signing_key, public_key, key_id) = test_keypair();
        let expired_anchor = TrustAnchor {
            key_id: key_id.clone(),
            public_key,
            valid_from: Utc::now() - chrono::Duration::hours(48),
            valid_until: Some(Utc::now() - chrono::Duration::hours(1)),
        };

        let bundle = sign_bundle(
            "permit(principal, action, resource);",
            1,
            None,
            &signing_key,
            &key_id,
        )
        .unwrap();

        let result = verify_bundle(&bundle, &[expired_anchor], None);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("not currently valid")
        );
    }

    #[test]
    fn test_trust_anchor_validity() {
        let future_anchor = TrustAnchor {
            key_id: "future".to_string(),
            public_key: vec![0u8; 32],
            valid_from: Utc::now() + chrono::Duration::hours(1),
            valid_until: None,
        };
        assert!(!future_anchor.is_valid());

        let current_anchor = TrustAnchor {
            key_id: "current".to_string(),
            public_key: vec![0u8; 32],
            valid_from: Utc::now() - chrono::Duration::hours(1),
            valid_until: Some(Utc::now() + chrono::Duration::hours(1)),
        };
        assert!(current_anchor.is_valid());

        let no_expiry = TrustAnchor {
            key_id: "forever".to_string(),
            public_key: vec![0u8; 32],
            valid_from: Utc::now() - chrono::Duration::hours(1),
            valid_until: None,
        };
        assert!(no_expiry.is_valid());
    }

    #[test]
    fn test_bundle_serialization_roundtrip() {
        let (signing_key, _, key_id) = test_keypair();

        let bundle = sign_bundle(
            "permit(principal, action, resource);",
            42,
            Some(Utc::now() + chrono::Duration::hours(24)),
            &signing_key,
            &key_id,
        )
        .unwrap();

        let json = serde_json::to_string(&bundle).unwrap();
        let restored: PolicyBundle = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.version, 42);
        assert_eq!(restored.policies, bundle.policies);
        assert_eq!(restored.signature, bundle.signature);
        assert_eq!(restored.signer_key_id, key_id);
    }
}
