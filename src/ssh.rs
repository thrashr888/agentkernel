//! SSH support for sandbox access.
//!
//! When `--ssh` is enabled, sandboxes get an sshd configured for
//! certificate-only authentication. Vault or a local CA signs
//! ephemeral client certificates.

use anyhow::{Context, Result, bail};
use ssh_key::{Algorithm, LineEnding, PrivateKey, certificate};

use crate::backend::FileInjection;

/// SSH configuration for a sandbox
#[derive(Debug, Clone)]
pub struct SshConfig {
    /// Enable SSH access
    pub enabled: bool,
    /// Port inside the sandbox for sshd (default: 22)
    pub port: u16,
    /// Host port to map sshd to (None = auto-assign)
    pub host_port: Option<u16>,
    /// Vault address for SSH CA (None = use built-in CA)
    pub vault_addr: Option<String>,
    /// Vault SSH secrets engine mount path
    pub vault_ssh_mount: String,
    /// Vault SSH role for signing
    pub vault_ssh_role: String,
    /// Certificate TTL
    pub cert_ttl: String,
    /// Username inside the sandbox
    pub user: String,
}

impl Default for SshConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: 22,
            host_port: None,
            vault_addr: None,
            vault_ssh_mount: "ssh".to_string(),
            vault_ssh_role: "agentkernel-client".to_string(),
            cert_ttl: "30m".to_string(),
            user: "sandbox".to_string(),
        }
    }
}

/// Generate an sshd_config string for certificate-only authentication.
pub fn generate_sshd_config(config: &SshConfig) -> String {
    format!(
        r#"# agentkernel sshd configuration — certificate-only auth
Port {port}
ListenAddress 0.0.0.0
Protocol 2

# Host key
HostKey /etc/ssh/ssh_host_ed25519_key

# Certificate-based authentication
TrustedUserCAKeys /etc/ssh/ca.pub
AuthorizedPrincipalsFile /etc/ssh/principals
PubkeyAuthentication yes

# Disable all other auth methods
PasswordAuthentication no
ChallengeResponseAuthentication no
KbdInteractiveAuthentication no
UsePAM no

# Disable root login
PermitRootLogin no

# Logging
LogLevel INFO

# Misc hardening
X11Forwarding no
PrintMotd no
AcceptEnv LANG LC_*
"#,
        port = config.port
    )
}

/// Generate an ed25519 CA keypair for the built-in (non-Vault) path.
///
/// Returns `(private_key_openssh, public_key_openssh)`.
pub fn generate_ca_keypair() -> Result<(String, String)> {
    let mut rng = rand::thread_rng();
    let private_key = PrivateKey::random(&mut rng, Algorithm::Ed25519)
        .context("Failed to generate CA ed25519 keypair")?;

    let private_pem = private_key
        .to_openssh(LineEnding::LF)
        .context("Failed to encode CA private key")?;

    let public_openssh = private_key
        .public_key()
        .to_openssh()
        .context("Failed to encode CA public key")?;

    Ok((private_pem.to_string(), public_openssh))
}

/// Generate an ed25519 host keypair for sshd.
///
/// Returns `(private_key_openssh, public_key_openssh)`.
fn generate_host_keypair() -> Result<(String, String)> {
    let mut rng = rand::thread_rng();
    let private_key = PrivateKey::random(&mut rng, Algorithm::Ed25519)
        .context("Failed to generate host ed25519 keypair")?;

    let private_pem = private_key
        .to_openssh(LineEnding::LF)
        .context("Failed to encode host private key")?;

    let public_openssh = private_key
        .public_key()
        .to_openssh()
        .context("Failed to encode host public key")?;

    Ok((private_pem.to_string(), public_openssh))
}

/// Generate a startup script that creates the user, sets permissions, and starts sshd.
fn generate_start_sshd_script(config: &SshConfig) -> String {
    format!(
        r#"#!/bin/sh
set -e

# Create the sandbox user if it doesn't exist
if ! id -u {user} >/dev/null 2>&1; then
    adduser -D -h /home/{user} -s /bin/sh {user} 2>/dev/null || \
        useradd -m -d /home/{user} -s /bin/sh {user} 2>/dev/null || true
fi

# Set up .ssh directory
mkdir -p /home/{user}/.ssh
chmod 700 /home/{user}/.ssh
chown -R {user}:{user} /home/{user}/.ssh 2>/dev/null || \
    chown -R {user} /home/{user}/.ssh

# Fix permissions on sshd files
chmod 600 /etc/ssh/ssh_host_ed25519_key
chmod 644 /etc/ssh/ssh_host_ed25519_key.pub
chmod 644 /etc/ssh/ca.pub
chmod 644 /etc/ssh/principals
chmod 644 /etc/ssh/sshd_config

# Generate host keys if sshd expects them (some distros require all types)
ssh-keygen -A 2>/dev/null || true

# Create privilege separation directory
mkdir -p /run/sshd 2>/dev/null || mkdir -p /var/run/sshd 2>/dev/null || true

# Start sshd in the background
/usr/sbin/sshd -f /etc/ssh/sshd_config -D &
echo "sshd started on port {port}"
"#,
        user = config.user,
        port = config.port,
    )
}

/// Build the list of files to inject into the sandbox for SSH support.
///
/// Includes sshd_config, CA public key, principals, host keypair, and startup script.
pub fn sshd_file_injections(
    ca_public_key: &str,
    ssh_config: &SshConfig,
) -> Result<Vec<FileInjection>> {
    let sshd_config_content = generate_sshd_config(ssh_config);
    let (host_private, host_public) = generate_host_keypair()?;
    let start_script = generate_start_sshd_script(ssh_config);

    let mut files = vec![
        FileInjection {
            content: sshd_config_content.into_bytes(),
            dest: "/etc/ssh/sshd_config".to_string(),
        },
        FileInjection {
            content: ca_public_key.as_bytes().to_vec(),
            dest: "/etc/ssh/ca.pub".to_string(),
        },
        FileInjection {
            content: format!("{}\n", ssh_config.user).into_bytes(),
            dest: "/etc/ssh/principals".to_string(),
        },
        FileInjection {
            content: host_private.into_bytes(),
            dest: "/etc/ssh/ssh_host_ed25519_key".to_string(),
        },
        FileInjection {
            content: host_public.into_bytes(),
            dest: "/etc/ssh/ssh_host_ed25519_key.pub".to_string(),
        },
        FileInjection {
            content: start_script.into_bytes(),
            dest: "/tmp/start-sshd.sh".to_string(),
        },
    ];

    // Placeholder for user .ssh directory — the startup script handles creation
    files.push(FileInjection {
        content: Vec::new(),
        dest: format!("/home/{}/.ssh/.keep", ssh_config.user),
    });

    Ok(files)
}

/// Sign a client public key with a local CA private key.
///
/// Returns the signed certificate in OpenSSH format.
pub fn sign_client_key_local(
    ca_private_key: &str,
    client_public_key: &str,
    principals: &[&str],
    ttl_secs: u64,
) -> Result<String> {
    let ca_key =
        PrivateKey::from_openssh(ca_private_key).context("Failed to parse CA private key")?;

    let client_pubkey = ssh_key::PublicKey::from_openssh(client_public_key)
        .context("Failed to parse client public key")?;

    let mut rng = rand::thread_rng();

    let valid_after = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("System time before UNIX epoch")?
        .as_secs();
    let valid_before = valid_after + ttl_secs;

    let mut builder = certificate::Builder::new_with_random_nonce(
        &mut rng,
        client_pubkey.key_data().clone(),
        valid_after,
        valid_before,
    )
    .context("Failed to create certificate builder")?;

    builder
        .cert_type(certificate::CertType::User)
        .context("Failed to set cert type")?;

    builder
        .key_id("agentkernel-client")
        .context("Failed to set key id")?;

    for principal in principals {
        builder
            .valid_principal(principal.to_string())
            .context("Failed to add principal")?;
    }

    let cert = builder
        .sign(&ca_key)
        .context("Failed to sign client certificate")?;

    cert.to_openssh().context("Failed to encode certificate")
}

/// Sign a client public key via Vault SSH secrets engine.
///
/// Makes an HTTP POST to `{vault_addr}/v1/{mount}/sign/{role}`.
/// Requires `VAULT_TOKEN` environment variable or explicit token.
#[allow(dead_code)]
pub async fn sign_client_key_vault(
    vault_addr: &str,
    vault_token: &str,
    ssh_config: &SshConfig,
    client_public_key: &str,
) -> Result<String> {
    // Vault SSH sign endpoint
    let url = format!(
        "{}/v1/{}/sign/{}",
        vault_addr.trim_end_matches('/'),
        ssh_config.vault_ssh_mount,
        ssh_config.vault_ssh_role
    );

    // Build the request body
    let body = serde_json::json!({
        "public_key": client_public_key,
        "valid_principals": ssh_config.user,
        "ttl": ssh_config.cert_ttl,
        "cert_type": "user",
    });

    // Use reqwest if available (behind enterprise/nomad feature), otherwise use hyper
    #[cfg(any(feature = "enterprise", feature = "nomad"))]
    {
        let client = reqwest::Client::new();
        let resp = client
            .post(&url)
            .header("X-Vault-Token", vault_token)
            .json(&body)
            .send()
            .await
            .context("Failed to contact Vault")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            bail!("Vault SSH sign failed ({}): {}", status, text);
        }

        let result: serde_json::Value = resp
            .json()
            .await
            .context("Failed to parse Vault response")?;

        let signed_key = result["data"]["signed_key"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Vault response missing signed_key"))?;

        Ok(signed_key.to_string())
    }

    #[cfg(not(any(feature = "enterprise", feature = "nomad")))]
    {
        let _ = (url, body, vault_token);
        bail!(
            "Vault SSH signing requires the 'enterprise' or 'nomad' feature \
             (for reqwest HTTP client). Rebuild with --features enterprise"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ssh_config_defaults() {
        let config = SshConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.port, 22);
        assert!(config.host_port.is_none());
        assert!(config.vault_addr.is_none());
        assert_eq!(config.vault_ssh_mount, "ssh");
        assert_eq!(config.vault_ssh_role, "agentkernel-client");
        assert_eq!(config.cert_ttl, "30m");
        assert_eq!(config.user, "sandbox");
    }

    #[test]
    fn test_generate_sshd_config_contains_directives() {
        let config = SshConfig::default();
        let sshd_config = generate_sshd_config(&config);

        assert!(sshd_config.contains("Port 22"));
        assert!(sshd_config.contains("TrustedUserCAKeys /etc/ssh/ca.pub"));
        assert!(sshd_config.contains("PasswordAuthentication no"));
        assert!(sshd_config.contains("PermitRootLogin no"));
        assert!(sshd_config.contains("AuthorizedPrincipalsFile /etc/ssh/principals"));
        assert!(sshd_config.contains("PubkeyAuthentication yes"));
        assert!(sshd_config.contains("HostKey /etc/ssh/ssh_host_ed25519_key"));
    }

    #[test]
    fn test_generate_sshd_config_custom_port() {
        let config = SshConfig {
            port: 2222,
            ..SshConfig::default()
        };
        let sshd_config = generate_sshd_config(&config);
        assert!(sshd_config.contains("Port 2222"));
        assert!(!sshd_config.contains("Port 22\n"));
    }

    #[test]
    fn test_generate_ca_keypair() {
        let (private_key, public_key) = generate_ca_keypair().unwrap();

        // Private key should be in OpenSSH PEM format
        assert!(private_key.contains("BEGIN OPENSSH PRIVATE KEY"));
        assert!(private_key.contains("END OPENSSH PRIVATE KEY"));

        // Public key should start with ssh-ed25519
        assert!(public_key.starts_with("ssh-ed25519 "));
    }

    #[test]
    fn test_sshd_file_injections_returns_correct_files() {
        let config = SshConfig::default();
        let (_, ca_pub) = generate_ca_keypair().unwrap();
        let files = sshd_file_injections(&ca_pub, &config).unwrap();

        let dests: Vec<&str> = files.iter().map(|f| f.dest.as_str()).collect();

        assert!(dests.contains(&"/etc/ssh/sshd_config"));
        assert!(dests.contains(&"/etc/ssh/ca.pub"));
        assert!(dests.contains(&"/etc/ssh/principals"));
        assert!(dests.contains(&"/etc/ssh/ssh_host_ed25519_key"));
        assert!(dests.contains(&"/etc/ssh/ssh_host_ed25519_key.pub"));
        assert!(dests.contains(&"/tmp/start-sshd.sh"));
        assert!(dests.contains(&"/home/sandbox/.ssh/.keep"));
    }

    #[test]
    fn test_sshd_file_injections_principals_content() {
        let config = SshConfig {
            user: "testuser".to_string(),
            ..SshConfig::default()
        };
        let (_, ca_pub) = generate_ca_keypair().unwrap();
        let files = sshd_file_injections(&ca_pub, &config).unwrap();

        let principals = files
            .iter()
            .find(|f| f.dest == "/etc/ssh/principals")
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&principals.content), "testuser\n");
    }

    #[test]
    fn test_sshd_file_injections_custom_user_path() {
        let config = SshConfig {
            user: "agent".to_string(),
            ..SshConfig::default()
        };
        let (_, ca_pub) = generate_ca_keypair().unwrap();
        let files = sshd_file_injections(&ca_pub, &config).unwrap();

        let dests: Vec<&str> = files.iter().map(|f| f.dest.as_str()).collect();
        assert!(dests.contains(&"/home/agent/.ssh/.keep"));
    }

    #[test]
    fn test_sign_client_key_local() {
        let (ca_priv, _ca_pub) = generate_ca_keypair().unwrap();

        // Generate a client keypair
        let mut rng = rand::thread_rng();
        let client_key = PrivateKey::random(&mut rng, Algorithm::Ed25519).unwrap();
        let client_pub = client_key.public_key().to_openssh().unwrap();

        let cert = sign_client_key_local(
            &ca_priv,
            &client_pub,
            &["sandbox"],
            1800, // 30 minutes
        )
        .unwrap();

        // Certificate should be parseable and in OpenSSH format
        assert!(cert.contains("ssh-ed25519-cert-v01@openssh.com"));
    }

    #[test]
    fn test_start_sshd_script_content() {
        let config = SshConfig {
            user: "myuser".to_string(),
            port: 2222,
            ..SshConfig::default()
        };
        let script = generate_start_sshd_script(&config);

        assert!(script.contains("#!/bin/sh"));
        assert!(script.contains("myuser"));
        assert!(script.contains("port 2222"));
        assert!(script.contains("chmod 600 /etc/ssh/ssh_host_ed25519_key"));
        assert!(script.contains("/usr/sbin/sshd"));
    }

    #[test]
    fn test_generate_host_keypair() {
        let (private_key, public_key) = generate_host_keypair().unwrap();
        assert!(private_key.contains("BEGIN OPENSSH PRIVATE KEY"));
        assert!(public_key.starts_with("ssh-ed25519 "));
    }
}
