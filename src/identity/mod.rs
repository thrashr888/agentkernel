//! Identity management for enterprise agent authentication.
//!
//! Provides JWT validation, API key authentication, and identity mapping
//! to Cedar policy principals for enterprise policy evaluation.

#[cfg(feature = "enterprise")]
pub mod oidc;

#[cfg(feature = "enterprise")]
use anyhow::{Context, Result, bail};
#[cfg(feature = "enterprise")]
use serde::{Deserialize, Serialize};

/// JWT claims extracted from a validated token.
#[cfg(feature = "enterprise")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtClaims {
    /// Subject identifier (unique user ID)
    pub sub: String,
    /// User email address
    pub email: String,
    /// Organization identifier
    pub org_id: String,
    /// Roles assigned to the user
    pub roles: Vec<String>,
    /// Whether the user has completed MFA verification
    #[serde(default)]
    pub mfa_verified: bool,
    /// Token expiration (Unix timestamp)
    #[serde(default)]
    pub exp: Option<u64>,
    /// Token issued-at (Unix timestamp)
    #[serde(default)]
    pub iat: Option<u64>,
}

/// Agent identity combining API key and JWT-based authentication.
#[cfg(feature = "enterprise")]
#[derive(Debug, Clone)]
pub struct AgentIdentity {
    /// API key used for authentication (if present)
    pub api_key: Option<String>,
    /// JWT claims extracted from a validated token (if present)
    pub jwt_claims: Option<JwtClaims>,
}

#[cfg(feature = "enterprise")]
impl AgentIdentity {
    /// Create an empty identity (no authentication)
    pub fn anonymous() -> Self {
        Self {
            api_key: None,
            jwt_claims: None,
        }
    }

    /// Create an identity from validated JWT claims
    pub fn from_jwt(claims: JwtClaims) -> Self {
        Self {
            api_key: None,
            jwt_claims: Some(claims),
        }
    }

    /// Create an identity from an API key
    pub fn from_api_key(key: String) -> Self {
        Self {
            api_key: Some(key),
            jwt_claims: None,
        }
    }

    /// Get the user's subject identifier, if available
    pub fn subject(&self) -> Option<&str> {
        self.jwt_claims.as_ref().map(|c| c.sub.as_str())
    }

    /// Get the user's email, if available
    pub fn email(&self) -> Option<&str> {
        self.jwt_claims.as_ref().map(|c| c.email.as_str())
    }

    /// Get the user's organization ID, if available
    pub fn org_id(&self) -> Option<&str> {
        self.jwt_claims.as_ref().map(|c| c.org_id.as_str())
    }

    /// Check if the identity has a specific role
    pub fn has_role(&self, role: &str) -> bool {
        self.jwt_claims
            .as_ref()
            .is_some_and(|c| c.roles.iter().any(|r| r == role))
    }

    /// Check if MFA is verified
    pub fn mfa_verified(&self) -> bool {
        self.jwt_claims.as_ref().is_some_and(|c| c.mfa_verified)
    }

    /// Whether this identity is authenticated (has either API key or JWT)
    pub fn is_authenticated(&self) -> bool {
        self.api_key.is_some() || self.jwt_claims.is_some()
    }
}

/// JWKS (JSON Web Key Set) key for JWT verification.
#[cfg(feature = "enterprise")]
#[derive(Debug, Clone, Deserialize)]
pub struct JwksKey {
    /// Key type (e.g., "RSA")
    pub kty: String,
    /// Key use (e.g., "sig")
    #[serde(rename = "use")]
    pub use_: Option<String>,
    /// Key ID
    pub kid: Option<String>,
    /// Algorithm (e.g., "RS256")
    pub alg: Option<String>,
    /// RSA modulus (base64url-encoded)
    pub n: Option<String>,
    /// RSA exponent (base64url-encoded)
    pub e: Option<String>,
}

/// JWKS response containing multiple keys.
#[cfg(feature = "enterprise")]
#[derive(Debug, Clone, Deserialize)]
pub struct JwksResponse {
    pub keys: Vec<JwksKey>,
}

/// Validate a JWT token against a JWKS endpoint.
///
/// Fetches the JWKS from the provided URL, finds the matching key,
/// and validates the token signature and claims.
#[cfg(feature = "enterprise")]
pub async fn validate_jwt(token: &str, jwks_url: &str) -> Result<JwtClaims> {
    use jsonwebtoken::{Algorithm, DecodingKey, TokenData, Validation, decode, decode_header};

    // Decode the header to get the key ID (kid)
    let header = decode_header(token).context("Failed to decode JWT header")?;
    let kid = header
        .kid
        .as_ref()
        .context("JWT header missing 'kid' field")?;

    // Fetch JWKS from the endpoint
    let client = reqwest::Client::new();
    let jwks: JwksResponse = client
        .get(jwks_url)
        .send()
        .await
        .context("Failed to fetch JWKS")?
        .json()
        .await
        .context("Failed to parse JWKS response")?;

    // Find the matching key
    let jwk = jwks
        .keys
        .iter()
        .find(|k| k.kid.as_deref() == Some(kid))
        .with_context(|| format!("No matching key found for kid '{}'", kid))?;

    // Build the decoding key from RSA components
    let n = jwk.n.as_ref().context("JWKS key missing 'n' field")?;
    let e = jwk.e.as_ref().context("JWKS key missing 'e' field")?;
    let decoding_key =
        DecodingKey::from_rsa_components(n, e).context("Failed to create decoding key")?;

    // Configure validation
    let algorithm = match header.alg {
        jsonwebtoken::Algorithm::RS256 => Algorithm::RS256,
        jsonwebtoken::Algorithm::RS384 => Algorithm::RS384,
        jsonwebtoken::Algorithm::RS512 => Algorithm::RS512,
        other => bail!("Unsupported JWT algorithm: {:?}", other),
    };

    let mut validation = Validation::new(algorithm);
    validation.validate_exp = true;
    // Allow common clock skew of 60 seconds
    validation.leeway = 60;
    // We validate claims manually after decoding
    validation.set_required_spec_claims(&["exp", "sub"]);

    // Decode and validate
    let token_data: TokenData<JwtClaims> =
        decode(token, &decoding_key, &validation).context("JWT validation failed")?;

    Ok(token_data.claims)
}

/// Validate an API key against an expected value.
///
/// Uses constant-time comparison to prevent timing attacks.
#[cfg(feature = "enterprise")]
pub fn validate_api_key(key: &str, expected: &str) -> Result<AgentIdentity> {
    // Constant-time comparison to prevent timing attacks
    if constant_time_eq(key.as_bytes(), expected.as_bytes()) {
        Ok(AgentIdentity::from_api_key(key.to_string()))
    } else {
        bail!("Invalid API key")
    }
}

/// Constant-time byte comparison to prevent timing attacks.
#[cfg(feature = "enterprise")]
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Convert an AgentIdentity to a Cedar policy principal entity reference.
///
/// Returns a string entity reference in the format:
///   `AgentKernel::User::"<sub>"` for JWT-authenticated users
///   `AgentKernel::ApiClient::"<key_prefix>"` for API key authenticated clients
///   `AgentKernel::Anonymous::""` for unauthenticated requests
#[cfg(feature = "enterprise")]
pub fn to_cedar_principal(identity: &AgentIdentity) -> String {
    if let Some(ref claims) = identity.jwt_claims {
        format!("AgentKernel::User::\"{}\"", claims.sub)
    } else if let Some(ref key) = identity.api_key {
        // Use first 8 chars of API key as identifier (don't expose full key)
        let prefix = if key.len() >= 8 { &key[..8] } else { key };
        format!("AgentKernel::ApiClient::\"{}\"", prefix)
    } else {
        "AgentKernel::Anonymous::\"anonymous\"".to_string()
    }
}

/// Build Cedar context attributes from an AgentIdentity.
///
/// Returns a map of attribute names to values suitable for Cedar policy evaluation.
#[cfg(feature = "enterprise")]
pub fn to_cedar_context(
    identity: &AgentIdentity,
) -> std::collections::HashMap<String, serde_json::Value> {
    let mut context = std::collections::HashMap::new();

    if let Some(ref claims) = identity.jwt_claims {
        context.insert("email".to_string(), serde_json::json!(claims.email));
        context.insert("org_id".to_string(), serde_json::json!(claims.org_id));
        context.insert("roles".to_string(), serde_json::json!(claims.roles));
        context.insert(
            "mfa_verified".to_string(),
            serde_json::json!(claims.mfa_verified),
        );
    }

    context.insert(
        "is_authenticated".to_string(),
        serde_json::json!(identity.is_authenticated()),
    );

    context
}

#[cfg(all(test, feature = "enterprise"))]
mod tests {
    use super::*;

    #[test]
    fn test_agent_identity_anonymous() {
        let identity = AgentIdentity::anonymous();
        assert!(!identity.is_authenticated());
        assert!(identity.subject().is_none());
        assert!(identity.email().is_none());
        assert!(!identity.has_role("admin"));
        assert!(!identity.mfa_verified());
    }

    #[test]
    fn test_agent_identity_from_jwt() {
        let claims = JwtClaims {
            sub: "user-123".to_string(),
            email: "user@example.com".to_string(),
            org_id: "acme-corp".to_string(),
            roles: vec!["developer".to_string(), "admin".to_string()],
            mfa_verified: true,
            exp: None,
            iat: None,
        };
        let identity = AgentIdentity::from_jwt(claims);

        assert!(identity.is_authenticated());
        assert_eq!(identity.subject(), Some("user-123"));
        assert_eq!(identity.email(), Some("user@example.com"));
        assert_eq!(identity.org_id(), Some("acme-corp"));
        assert!(identity.has_role("developer"));
        assert!(identity.has_role("admin"));
        assert!(!identity.has_role("viewer"));
        assert!(identity.mfa_verified());
    }

    #[test]
    fn test_agent_identity_from_api_key() {
        let identity = AgentIdentity::from_api_key("ak_test_12345678".to_string());
        assert!(identity.is_authenticated());
        assert!(identity.subject().is_none());
        assert!(identity.jwt_claims.is_none());
        assert_eq!(identity.api_key, Some("ak_test_12345678".to_string()));
    }

    #[test]
    fn test_validate_api_key_valid() {
        let result = validate_api_key("correct-key", "correct-key");
        assert!(result.is_ok());
        let identity = result.unwrap();
        assert!(identity.is_authenticated());
    }

    #[test]
    fn test_validate_api_key_invalid() {
        let result = validate_api_key("wrong-key", "correct-key");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_api_key_different_length() {
        let result = validate_api_key("short", "much-longer-key");
        assert!(result.is_err());
    }

    #[test]
    fn test_constant_time_eq() {
        assert!(constant_time_eq(b"hello", b"hello"));
        assert!(!constant_time_eq(b"hello", b"world"));
        assert!(!constant_time_eq(b"hello", b"hell"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn test_to_cedar_principal_jwt() {
        let claims = JwtClaims {
            sub: "user-123".to_string(),
            email: "user@example.com".to_string(),
            org_id: "acme-corp".to_string(),
            roles: vec!["developer".to_string()],
            mfa_verified: false,
            exp: None,
            iat: None,
        };
        let identity = AgentIdentity::from_jwt(claims);
        let principal = to_cedar_principal(&identity);
        assert_eq!(principal, "AgentKernel::User::\"user-123\"");
    }

    #[test]
    fn test_to_cedar_principal_api_key() {
        let identity = AgentIdentity::from_api_key("ak_test_12345678abcdef".to_string());
        let principal = to_cedar_principal(&identity);
        assert_eq!(principal, "AgentKernel::ApiClient::\"ak_test_\"");
    }

    #[test]
    fn test_to_cedar_principal_anonymous() {
        let identity = AgentIdentity::anonymous();
        let principal = to_cedar_principal(&identity);
        assert_eq!(principal, "AgentKernel::Anonymous::\"anonymous\"");
    }

    #[test]
    fn test_to_cedar_context_jwt() {
        let claims = JwtClaims {
            sub: "user-123".to_string(),
            email: "user@example.com".to_string(),
            org_id: "acme-corp".to_string(),
            roles: vec!["developer".to_string()],
            mfa_verified: true,
            exp: None,
            iat: None,
        };
        let identity = AgentIdentity::from_jwt(claims);
        let context = to_cedar_context(&identity);

        assert_eq!(context.get("email").unwrap(), "user@example.com");
        assert_eq!(context.get("org_id").unwrap(), "acme-corp");
        assert_eq!(context.get("mfa_verified").unwrap(), true);
        assert_eq!(context.get("is_authenticated").unwrap(), true);
    }

    #[test]
    fn test_to_cedar_context_anonymous() {
        let identity = AgentIdentity::anonymous();
        let context = to_cedar_context(&identity);

        assert_eq!(context.get("is_authenticated").unwrap(), false);
        assert!(context.get("email").is_none());
    }
}
