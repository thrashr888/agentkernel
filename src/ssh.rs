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

/// Report a managed Docker sandbox's SSH URL on this host. Does not connect
/// to the SSH endpoint or issue credentials.
pub fn sandbox_ssh_url_for_state(state: &crate::vmm::SandboxState) -> Result<String> {
    use crate::backend::BackendType;

    let name = &state.name;
    crate::validation::validate_sandbox_name(name)?;
    if !state.ssh_enabled {
        bail!("SSH is not enabled on sandbox '{name}'. Recreate it with --ssh.");
    }
    if state.backend != Some(BackendType::Docker) {
        bail!("SSH URL lookup currently supports local Docker sandboxes only.");
    }
    let expected_port = state
        .ssh_host_port
        .filter(|&port| port != 0)
        .context("No managed SSH host port has been recorded for this sandbox.")?;
    let expected = state
        .docker_ssh_launch
        .as_ref()
        .context("No verified managed Docker SSH launch has been recorded for this sandbox.")?;
    if expected.port != expected_port {
        bail!("The saved port does not match the managed Docker SSH launch.");
    }
    let endpoint = local_docker_endpoint()?;
    expected.verify_ready(&endpoint, name)?;
    Ok(format!("ssh://sandbox@127.0.0.1:{expected_port}"))
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct DockerSshLaunch {
    id: String,
    started_at: String,
    endpoint: String,
    pub(crate) port: u16,
}

fn full_docker_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(crate) fn pinned_docker_command(endpoint: &str) -> std::process::Command {
    let mut command = std::process::Command::new("docker");
    command
        .env_remove("DOCKER_CONTEXT")
        .env_remove("DOCKER_HOST")
        .args(["--host", endpoint]);
    command
}

impl DockerSshLaunch {
    pub(crate) fn observe(endpoint: &str, name: &str, id: &str) -> Result<Self> {
        anyhow::ensure!(full_docker_id(id), "Invalid Docker container ID");
        let output = pinned_docker_command(endpoint)
            .args([
                "inspect",
                "--type",
                "container",
                "--format",
                r#"{{if and .State.Running (not .State.Paused) (not .State.Restarting)}}{{.Id}} {{.State.StartedAt}} {{.Name}} {{json (index .NetworkSettings.Ports "22/tcp")}}{{end}}"#,
                id,
            ])
            .output()
            .context("Could not inspect exact Docker SSH container")?;
        anyhow::ensure!(
            output.status.success(),
            "Exact Docker SSH container is not available"
        );
        Self::parse(endpoint, name, id, &String::from_utf8_lossy(&output.stdout))
    }

    fn parse(endpoint: &str, name: &str, id: &str, output: &str) -> Result<Self> {
        let mut parts = output.trim().splitn(4, ' ');
        let observed_id = parts.next().context("Missing Docker container ID")?;
        let started = parts.next().context("Missing Docker start generation")?;
        let observed_name = parts.next().context("Missing Docker container name")?;
        let bindings = parts.next().context("Missing Docker SSH binding")?;
        anyhow::ensure!(
            full_docker_id(id)
                && id == observed_id
                && observed_name == format!("/agentkernel-{name}"),
            "Docker SSH container identity or name does not match",
        );
        let start_time = chrono::DateTime::parse_from_rfc3339(started)
            .context("Invalid Docker start generation")?;
        anyhow::ensure!(
            start_time.timestamp() > 0,
            "Docker container has no valid start"
        );
        let raw: serde_json::Value = serde_json::from_str(bindings)?;
        let port = raw
            .as_array()
            .and_then(|bindings| bindings.first())
            .and_then(|binding| binding.get("HostPort"))
            .and_then(serde_json::Value::as_str)
            .and_then(|port| port.parse::<u16>().ok())
            .context("Missing Docker SSH host port")?;
        anyhow::ensure!(
            port != 0 && private_docker_ssh_url_from_bindings(bindings, port)?.is_some(),
            "Docker SSH route is not exclusively bound to loopback"
        );
        Ok(Self {
            id: id.into(),
            started_at: started.into(),
            endpoint: endpoint.into(),
            port,
        })
    }

    #[cfg(all(
        target_os = "macos",
        target_arch = "aarch64",
        feature = "docker-rex-local"
    ))]
    pub(crate) fn rex_owner_identity(&self) -> (&str, &str, &str) {
        (&self.id, &self.started_at, &self.endpoint)
    }

    fn require_same(&self, observed: &Self) -> Result<()> {
        anyhow::ensure!(
            self == observed,
            "Docker SSH container generation or route changed"
        );
        Ok(())
    }

    pub(crate) fn verify_ready(&self, endpoint: &str, name: &str) -> Result<()> {
        self.verify_ready_with(
            endpoint,
            |id| Self::observe(endpoint, name, id),
            |id| {
                let status = pinned_docker_command(endpoint)
                    .args(["exec", id, "sh", "-c", DOCKER_SSHD_READY])
                    .output()
                    .context("Could not verify Docker SSH readiness")?;
                anyhow::ensure!(
                    status.status.success(),
                    "Docker sandbox '{name}' has no ready managed SSH process."
                );
                Ok(())
            },
        )
    }

    fn verify_ready_with(
        &self,
        endpoint: &str,
        mut observe: impl FnMut(&str) -> Result<Self>,
        check_listener: impl FnOnce(&str) -> Result<()>,
    ) -> Result<()> {
        anyhow::ensure!(
            self.endpoint == endpoint,
            "The local Docker SSH endpoint changed"
        );
        self.require_same(&observe(&self.id)?)?;
        check_listener(&self.id)?;
        // Detect a restart, rename or remap during the guest observation.
        self.require_same(&observe(&self.id)?)
    }
}

pub(crate) fn local_docker_endpoint() -> Result<String> {
    // DOCKER_CONTEXT wins over DOCKER_HOST. Without an override, inspect the
    // effective context's endpoint without contacting the daemon.
    let docker_context = std::env::var_os("DOCKER_CONTEXT").filter(|v| !v.is_empty());
    let endpoint = match std::env::var_os("DOCKER_HOST").filter(|v| !v.is_empty()) {
        Some(host) if docker_context.is_none() => host
            .into_string()
            .map_err(|_| anyhow::anyhow!("Docker host is not valid UTF-8"))?,
        _ => {
            let mut command = std::process::Command::new("docker");
            command.args([
                "context",
                "inspect",
                "--format",
                "{{.Endpoints.docker.Host}}",
            ]);
            if let Some(context) = docker_context {
                command.arg("--").arg(context);
            }
            let output = command
                .output()
                .context("Could not read the effective Docker context")?;
            if !output.status.success() {
                bail!("Could not determine the effective Docker endpoint");
            }
            String::from_utf8(output.stdout)
                .context("Docker endpoint is not valid UTF-8")?
                .trim()
                .to_owned()
        }
    };
    if !is_local_docker_endpoint(&endpoint) {
        bail!(
            "SSH URL lookup requires a local Docker Unix socket; remote Docker contexts are not supported."
        );
    }
    Ok(endpoint)
}

fn is_local_docker_endpoint(endpoint: &str) -> bool {
    endpoint
        .strip_prefix("unix://")
        .is_some_and(|path| path.starts_with('/') && !path.starts_with("//") && path.len() > 1)
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PortBinding {
    #[serde(rename = "HostIp")]
    host_ip: String,
    host_port: String,
}

fn private_docker_ssh_url_from_bindings(
    bindings: &str,
    expected_port: u16,
) -> Result<Option<String>> {
    let bindings: Option<Vec<PortBinding>> = serde_json::from_str(bindings)
        .context("Could not read published Docker SSH port metadata")?;
    let Some([binding]) = bindings.as_deref() else {
        return Ok(None);
    };
    let literal_host = match binding.host_ip.as_str() {
        "127.0.0.1" => "127.0.0.1",
        "::1" => "[::1]",
        _ => return Ok(None),
    };
    if expected_port == 0 || binding.host_port != expected_port.to_string() {
        return Ok(None);
    }
    Ok(Some(format!(
        "ssh://sandbox@{literal_host}:{expected_port}"
    )))
}

/// Generate an sshd_config string for certificate-only authentication.
pub fn generate_sshd_config(config: &SshConfig) -> String {
    format!(
        r#"# agentkernel sshd configuration — certificate-only auth
Port {port}
ListenAddress 0.0.0.0

# Host key
HostKey /etc/ssh/ssh_host_ed25519_key

# Certificate-based authentication
TrustedUserCAKeys /etc/ssh/ca.pub
AuthorizedPrincipalsFile /etc/ssh/principals
PubkeyAuthentication yes

# Disable all other auth methods
PasswordAuthentication no
KbdInteractiveAuthentication no

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
    let mut rng = ssh_key::rand_core::OsRng;
    let private_key = PrivateKey::random(&mut rng, Algorithm::Ed25519)
        .map_err(anyhow::Error::msg)
        .context("Failed to generate CA ed25519 keypair")?;

    let private_pem = private_key
        .to_openssh(LineEnding::LF)
        .map_err(anyhow::Error::msg)
        .context("Failed to encode CA private key")?;

    let public_openssh = private_key
        .public_key()
        .to_openssh()
        .map_err(anyhow::Error::msg)
        .context("Failed to encode CA public key")?;

    Ok((private_pem.to_string(), public_openssh))
}

/// Generate an ed25519 host keypair for sshd.
///
/// Returns `(private_key_openssh, public_key_openssh)`.
fn generate_host_keypair() -> Result<(String, String)> {
    let mut rng = ssh_key::rand_core::OsRng;
    let private_key = PrivateKey::random(&mut rng, Algorithm::Ed25519)
        .map_err(anyhow::Error::msg)
        .context("Failed to generate host ed25519 keypair")?;

    let private_pem = private_key
        .to_openssh(LineEnding::LF)
        .map_err(anyhow::Error::msg)
        .context("Failed to encode host private key")?;

    let public_openssh = private_key
        .public_key()
        .to_openssh()
        .map_err(anyhow::Error::msg)
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

# Unlock the account for SSH cert auth.
# adduser -D creates a locked account (shadow password '!').
# OpenSSH without PAM rejects locked accounts even for cert/pubkey auth.
passwd -u {user} 2>/dev/null || \
    sed -i 's/^{user}:!/{user}:/' /etc/shadow 2>/dev/null || true

# Set up .ssh directory and clean login profile
mkdir -p /home/{user}/.ssh
chmod 700 /home/{user}/.ssh
touch /home/{user}/.hushlogin
cat > /home/{user}/.profile << 'PROFILE'
export PS1="agentkernel:$(basename "$PWD")\$ "
PROFILE
chown -R {user}:{user} /home/{user} 2>/dev/null || \
    chown -R {user} /home/{user}

# Fix ownership and permissions on sshd files
# (docker cp preserves host UID; sshd StrictModes requires root ownership)
chown root:root /etc/ssh/sshd_config /etc/ssh/ssh_host_ed25519_key \
    /etc/ssh/ssh_host_ed25519_key.pub /etc/ssh/ca.pub /etc/ssh/principals
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

/// Validate the managed PID against the actual Linux process and listener.
pub(crate) const DOCKER_SSHD_READY: &str = r#"set -e
pid=$(cat /run/sshd/agentkernel.pid)
case "$pid" in ""|*[!0-9]*|0|1) exit 1;; esac
test "$(readlink "/proc/$pid/exe")" = /usr/sbin/sshd
# The managed listener uses port 22. Confirm that its actual socket is listening.
set -- $(awk '$2 ~ /:0016$/ && $4 == "0A" { print $10 }' /proc/net/tcp /proc/net/tcp6)
for inode do
    for fd in /proc/"$pid"/fd/*; do
        if [ "$(readlink "$fd")" = "socket:[$inode]" ]; then exit 0; fi
    done
done
exit 1"#;

pub(crate) fn require_docker_sshd_start(result: crate::backend::ExecResult) -> Result<()> {
    if result.exit_code != 0 {
        bail!(
            "Managed Docker sshd setup failed (exit {}): {}",
            result.exit_code,
            result.stderr.trim()
        );
    }
    Ok(())
}

pub(crate) fn docker_sshd_file_injections(
    ca: &str,
    config: &SshConfig,
) -> Result<Vec<FileInjection>> {
    let mut files = sshd_file_injections(ca, config)?;
    files.retain(|file| !file.dest.ends_with("/.ssh/.keep"));
    for file in &mut files {
        match file.dest.as_str() {
            "/etc/ssh/sshd_config" => {
                file.content
                    .extend_from_slice(b"\nPidFile /run/sshd/agentkernel.pid\n");
            }
            "/tmp/start-sshd.sh" => {
                file.content = format!(r#"#!/bin/sh
set -e
if ! id -u {user} >/dev/null 2>&1; then
    if command -v useradd >/dev/null 2>&1; then
        useradd -m -d /home/{user} -s /bin/sh -p '*' {user}
    else
        adduser -D -h /home/{user} -s /bin/sh {user}
        printf '%s\n' '{user}:*' | chpasswd -e
    fi
fi
id -u {user} >/dev/null
test ! -L /run/sshd
mkdir -p /run/sshd
chown root:root /run/sshd /etc/ssh/sshd_config /etc/ssh/ssh_host_ed25519_key /etc/ssh/ca.pub /etc/ssh/principals
chmod 755 /run/sshd
chmod 600 /etc/ssh/ssh_host_ed25519_key
chmod 644 /etc/ssh/sshd_config /etc/ssh/ca.pub /etc/ssh/principals
/usr/sbin/sshd -t -f /etc/ssh/sshd_config
/usr/sbin/sshd -f /etc/ssh/sshd_config
{DOCKER_SSHD_READY}
"#, user = config.user).into_bytes();
            }
            _ => {}
        }
    }
    Ok(files)
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
    let ca_key = PrivateKey::from_openssh(ca_private_key)
        .map_err(anyhow::Error::msg)
        .context("Failed to parse CA private key")?;

    let client_pubkey = ssh_key::PublicKey::from_openssh(client_public_key)
        .map_err(anyhow::Error::msg)
        .context("Failed to parse client public key")?;

    let mut rng = ssh_key::rand_core::OsRng;

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
    .map_err(anyhow::Error::msg)
    .context("Failed to create certificate builder")?;

    builder
        .cert_type(certificate::CertType::User)
        .map_err(anyhow::Error::msg)
        .context("Failed to set cert type")?;

    builder
        .key_id("agentkernel-client")
        .map_err(anyhow::Error::msg)
        .context("Failed to set key id")?;

    for principal in principals {
        builder
            .valid_principal(principal.to_string())
            .map_err(anyhow::Error::msg)
            .context("Failed to add principal")?;
    }

    // Add standard extensions (ssh-keygen includes these by default).
    // Without permit-pty, sshd rejects PTY allocation requests.
    for ext in &[
        "permit-X11-forwarding",
        "permit-agent-forwarding",
        "permit-port-forwarding",
        "permit-pty",
        "permit-user-rc",
    ] {
        builder
            .extension(*ext, "")
            .map_err(anyhow::Error::msg)
            .context("Failed to add extension")?;
    }

    let cert = builder
        .sign(&ca_key)
        .map_err(anyhow::Error::msg)
        .context("Failed to sign client certificate")?;

    cert.to_openssh()
        .map_err(anyhow::Error::msg)
        .context("Failed to encode certificate")
}

/// Generate an ephemeral ed25519 client keypair.
///
/// Returns `(private_key_openssh, public_key_openssh)`.
pub fn generate_client_keypair() -> Result<(String, String)> {
    let mut rng = ssh_key::rand_core::OsRng;
    let private_key = PrivateKey::random(&mut rng, Algorithm::Ed25519)
        .map_err(anyhow::Error::msg)
        .context("Failed to generate client ed25519 keypair")?;

    let private_pem = private_key
        .to_openssh(LineEnding::LF)
        .map_err(anyhow::Error::msg)
        .context("Failed to encode client private key")?;

    let public_openssh = private_key
        .public_key()
        .to_openssh()
        .map_err(anyhow::Error::msg)
        .context("Failed to encode client public key")?;

    Ok((private_pem.to_string(), public_openssh))
}

/// Parse a TTL string (e.g. "30m", "1h", "5m", "2h30m") to seconds.
///
/// Supports:
/// - `Ns` — seconds
/// - `Nm` — minutes
/// - `Nh` — hours
/// - plain integer — treated as seconds
pub fn parse_ttl_to_secs(ttl: &str) -> Result<u64> {
    let ttl = ttl.trim();
    if ttl.is_empty() {
        bail!("TTL string is empty");
    }

    // Plain integer = seconds
    if let Ok(secs) = ttl.parse::<u64>() {
        return Ok(secs);
    }

    let mut total: u64 = 0;
    let mut current_num = String::new();

    for ch in ttl.chars() {
        if ch.is_ascii_digit() {
            current_num.push(ch);
        } else {
            if current_num.is_empty() {
                bail!("Invalid TTL format: unexpected '{}' in \"{}\"", ch, ttl);
            }
            let n: u64 = current_num
                .parse()
                .context("Invalid number in TTL string")?;
            current_num.clear();
            match ch {
                's' => total += n,
                'm' => total += n * 60,
                'h' => total += n * 3600,
                _ => bail!("Unknown TTL unit '{}' in \"{}\"", ch, ttl),
            }
        }
    }

    // If there are trailing digits with no unit, reject (ambiguous)
    if !current_num.is_empty() {
        bail!(
            "Invalid TTL format: trailing digits without unit in \"{}\"",
            ttl
        );
    }

    if total == 0 {
        bail!("TTL resolves to 0 seconds: \"{}\"", ttl);
    }

    Ok(total)
}

/// Convenience wrapper that routes to Vault or local signing based on config.
///
/// If `ssh_config.vault_addr` is set, delegates to `sign_client_key_vault`
/// (requires an async runtime and the enterprise/nomad feature).
/// Otherwise, uses `sign_client_key_local` with the provided `ca_private_key`.
pub fn sign_client_key(
    ssh_config: &SshConfig,
    ca_private_key: Option<&str>,
    client_public_key: &str,
    principals: &[&str],
    ttl_secs: u64,
) -> Result<String> {
    if ssh_config.vault_addr.is_some() {
        // Vault path — we can't call async from a sync function directly.
        // Callers that need Vault signing should use sign_client_key_vault() directly
        // in an async context. For now, we bail to indicate the intent.
        bail!(
            "Vault SSH signing requires calling sign_client_key_vault() \
             in an async context. Set vault_addr=None to use local CA signing."
        );
    }

    let ca_key = ca_private_key.ok_or_else(|| {
        anyhow::anyhow!("Local CA signing requires a CA private key (ca_private_key is None)")
    })?;

    sign_client_key_local(ca_key, client_public_key, principals, ttl_secs)
}

/// Fetch the CA public key from Vault's SSH secrets engine.
///
/// Makes a GET request to `{vault_addr}/v1/{mount}/config/ca`.
/// This is needed so that Vault-signed certificates can be verified by
/// sshd inside sandboxes (inject the returned public key as `ca.pub`).
#[cfg(any(feature = "enterprise", feature = "nomad"))]
#[allow(dead_code)]
pub async fn get_vault_ca_public_key(
    vault_addr: &str,
    vault_token: &str,
    ssh_config: &SshConfig,
) -> Result<String> {
    let url = format!(
        "{}/v1/{}/config/ca",
        vault_addr.trim_end_matches('/'),
        ssh_config.vault_ssh_mount,
    );

    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .header("X-Vault-Token", vault_token)
        .send()
        .await
        .context("Failed to contact Vault for CA public key")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        bail!("Vault CA public key fetch failed ({}): {}", status, text);
    }

    let result: serde_json::Value = resp
        .json()
        .await
        .context("Failed to parse Vault CA response")?;

    let public_key = result["data"]["public_key"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Vault response missing data.public_key"))?;

    Ok(public_key.to_string())
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
    #[test]
    fn ssh_url_rejects_remote_and_ambiguous_docker_endpoints() {
        assert!(is_local_docker_endpoint("unix:///var/run/docker.sock"));
        for endpoint in [
            "ssh://build.example.com",
            "tcp://127.0.0.1:2375",
            "https://build.example.com:2376",
            "unix://build.example.com/run/docker.sock",
            "unix:relative",
            "",
        ] {
            assert!(!is_local_docker_endpoint(endpoint), "{endpoint}");
        }
    }

    #[test]
    fn docker_private_ssh_url_never_rewrites_a_wildcard_or_ambiguous_binding() {
        for (ip, formatted) in [("127.0.0.1", "127.0.0.1"), ("::1", "[::1]")] {
            let one = serde_json::json!([{"HostIp":ip,"HostPort":"2200"}]).to_string();
            assert_eq!(
                private_docker_ssh_url_from_bindings(&one, 2200).unwrap(),
                Some(format!("ssh://sandbox@{formatted}:2200"))
            );
            assert_eq!(
                private_docker_ssh_url_from_bindings(&one, 2201).unwrap(),
                None
            );
        }
        for ip in ["", "0.0.0.0", "::", "192.0.2.1"] {
            let one = serde_json::json!([{"HostIp":ip,"HostPort":"2200"}]).to_string();
            assert_eq!(
                private_docker_ssh_url_from_bindings(&one, 2200).unwrap(),
                None
            );
        }
        for input in [
            "null",
            "[]",
            r#"[{"HostIp":"127.0.0.1","HostPort":"2200"},{"HostIp":"0.0.0.0","HostPort":"2200"}]"#,
        ] {
            assert_eq!(
                private_docker_ssh_url_from_bindings(input, 2200).unwrap(),
                None
            );
        }
    }

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
    fn test_ssh_signing_keys_keep_zeroize_on_drop() {
        // This dalek version implements Drop only when its zeroize feature is
        // enabled. Do not rely on the unrelated JWT provider to enable it.
        assert!(std::mem::needs_drop::<ssh_ed25519_dalek::SigningKey>());
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
        let (ca_priv, ca_pub) = generate_ca_keypair().unwrap();

        // Generate a client keypair
        let mut rng = ssh_key::rand_core::OsRng;
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

        // Verify the cert includes permit-pty and other standard extensions
        let parsed = ssh_key::Certificate::from_openssh(&cert).unwrap();
        let ca_public = ssh_key::PublicKey::from_openssh(&ca_pub).unwrap();
        let fingerprint = ca_public.fingerprint(ssh_key::HashAlg::Sha256);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        parsed.validate_at(now, [&fingerprint]).unwrap();
        let (_, unrelated_ca) = generate_ca_keypair().unwrap();
        let unrelated_fingerprint = ssh_key::PublicKey::from_openssh(&unrelated_ca)
            .unwrap()
            .fingerprint(ssh_key::HashAlg::Sha256);
        assert!(parsed.validate_at(now, [&unrelated_fingerprint]).is_err());
        assert!(parsed.validate_at(now + 3600, [&fingerprint]).is_err());
        let extensions = parsed.extensions();
        assert!(
            extensions.get("permit-pty").is_some(),
            "Certificate must include permit-pty extension for PTY allocation"
        );
        assert!(extensions.get("permit-port-forwarding").is_some());
        assert!(extensions.get("permit-agent-forwarding").is_some());
    }

    #[test]
    fn test_sign_client_key_preserves_invalid_key_errors() {
        let error = sign_client_key_local("invalid", "invalid", &["sandbox"], 1800).unwrap_err();
        assert_eq!(error.to_string(), "Failed to parse CA private key");
        assert!(error.chain().count() > 1);
        let (ca_priv, _) = generate_ca_keypair().unwrap();
        let error = sign_client_key_local(&ca_priv, "invalid", &["sandbox"], 1800).unwrap_err();
        assert_eq!(error.to_string(), "Failed to parse client public key");
        assert!(error.chain().count() > 1);
    }

    #[test]
    fn docker_ssh_generation_rejects_replacement_and_restart_at_same_port() {
        const ENDPOINT: &str = "unix:///private/tmp/test.sock";
        let id = "a".repeat(64);
        let newcomer = "b".repeat(64);
        let row = |id: &str, stamp: &str| {
            format!(
                r#"{id} {stamp} /agentkernel-test [{{"HostIp":"127.0.0.1","HostPort":"42022"}}]"#
            )
        };
        let old_stamp = "2026-11-22T01:00:00.012345678Z";
        let new_stamp = "2026-11-22T01:00:00.012345679Z";
        let old = DockerSshLaunch::parse(ENDPOINT, "test", &id, &row(&id, old_stamp)).unwrap();
        let restarted =
            DockerSshLaunch::parse(ENDPOINT, "test", &id, &row(&id, new_stamp)).unwrap();
        let replacement =
            DockerSshLaunch::parse(ENDPOINT, "test", &newcomer, &row(&newcomer, old_stamp))
                .unwrap();
        assert!(DockerSshLaunch::parse(ENDPOINT, "test", &id, &row(&newcomer, old_stamp)).is_err());
        for candidate in [&restarted, &replacement] {
            assert!(
                old.verify_ready_with(
                    ENDPOINT,
                    |requested| {
                        assert_eq!(requested, id);
                        Ok(candidate.clone())
                    },
                    |_| panic!("must not probe an altered or unowned launch"),
                )
                .is_err()
            );
        }
        let mut observations = [old.clone(), restarted].into_iter();
        assert!(
            old.verify_ready_with(ENDPOINT, |_| Ok(observations.next().unwrap()), |_| Ok(()))
                .is_err()
        );
        assert!(
            old.verify_ready_with(
                ENDPOINT,
                |_| Ok(old.clone()),
                |requested| {
                    assert_eq!(requested, id);
                    Ok(())
                }
            )
            .is_ok()
        );
    }

    #[test]
    fn docker_provisioning_uses_a_dedicated_account_and_verified_listener() {
        let files = docker_sshd_file_injections("synthetic-ca", &SshConfig::default()).unwrap();
        assert!(!files.iter().any(|file| file.dest.starts_with("/home/")));
        let get = |dest| {
            String::from_utf8_lossy(&files.iter().find(|file| file.dest == dest).unwrap().content)
        };
        let config = get("/etc/ssh/sshd_config");
        assert!(config.contains("PidFile /run/sshd/agentkernel.pid"));
        assert!(config.contains("PasswordAuthentication no"));
        let script = get("/tmp/start-sshd.sh");
        assert!(script.contains("if ! id -u sandbox"));
        assert!(script.contains(" -p '*' sandbox"));
        assert!(!script.contains("/etc/shadow"));
        assert!(script.contains("test ! -L /run/sshd"));
        let validate = script.find("/usr/sbin/sshd -t").unwrap();
        let start = script.find("/usr/sbin/sshd -f").unwrap();
        assert!(validate < start && script[start..].contains(DOCKER_SSHD_READY));
    }

    #[test]
    fn docker_startup_failure_preserves_guest_error() {
        use crate::backend::ExecResult;
        assert!(require_docker_sshd_start(ExecResult::success(String::new())).is_ok());
        let result = ExecResult {
            exit_code: 17,
            stdout: String::new(),
            stderr: "synthetic boot failure".into(),
        };
        let error = require_docker_sshd_start(result).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Managed Docker sshd setup failed (exit 17): synthetic boot failure"
        );
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

    #[test]
    fn test_generate_client_keypair() {
        let (private_key, public_key) = generate_client_keypair().unwrap();

        // Private key should be in OpenSSH PEM format
        assert!(private_key.contains("BEGIN OPENSSH PRIVATE KEY"));
        assert!(private_key.contains("END OPENSSH PRIVATE KEY"));

        // Public key should start with ssh-ed25519
        assert!(public_key.starts_with("ssh-ed25519 "));
    }

    #[test]
    fn test_generate_client_keypair_is_unique() {
        let (priv1, pub1) = generate_client_keypair().unwrap();
        let (priv2, pub2) = generate_client_keypair().unwrap();
        // Each call should produce a different keypair
        assert_ne!(priv1, priv2);
        assert_ne!(pub1, pub2);
    }

    #[test]
    fn test_parse_ttl_to_secs_minutes() {
        assert_eq!(parse_ttl_to_secs("30m").unwrap(), 1800);
        assert_eq!(parse_ttl_to_secs("5m").unwrap(), 300);
        assert_eq!(parse_ttl_to_secs("1m").unwrap(), 60);
    }

    #[test]
    fn test_parse_ttl_to_secs_hours() {
        assert_eq!(parse_ttl_to_secs("1h").unwrap(), 3600);
        assert_eq!(parse_ttl_to_secs("2h").unwrap(), 7200);
    }

    #[test]
    fn test_parse_ttl_to_secs_seconds() {
        assert_eq!(parse_ttl_to_secs("90s").unwrap(), 90);
        assert_eq!(parse_ttl_to_secs("1s").unwrap(), 1);
    }

    #[test]
    fn test_parse_ttl_to_secs_combined() {
        assert_eq!(parse_ttl_to_secs("1h30m").unwrap(), 5400);
        assert_eq!(parse_ttl_to_secs("2h15m30s").unwrap(), 8130);
    }

    #[test]
    fn test_parse_ttl_to_secs_plain_integer() {
        assert_eq!(parse_ttl_to_secs("3600").unwrap(), 3600);
        assert_eq!(parse_ttl_to_secs("0").unwrap(), 0);
    }

    #[test]
    fn test_parse_ttl_to_secs_invalid() {
        assert!(parse_ttl_to_secs("").is_err());
        assert!(parse_ttl_to_secs("abc").is_err());
        assert!(parse_ttl_to_secs("30x").is_err());
        assert!(parse_ttl_to_secs("m30").is_err());
    }

    #[test]
    fn test_parse_ttl_to_secs_trailing_digits_rejected() {
        // "30m15" has trailing digits without a unit — ambiguous
        assert!(parse_ttl_to_secs("30m15").is_err());
    }

    #[test]
    fn test_sign_client_key_local_routing() {
        let (ca_priv, _) = generate_ca_keypair().unwrap();
        let (_, client_pub) = generate_client_keypair().unwrap();

        let config = SshConfig::default();

        // Local signing should work
        let cert =
            sign_client_key(&config, Some(&ca_priv), &client_pub, &["sandbox"], 1800).unwrap();
        assert!(cert.contains("ssh-ed25519-cert-v01@openssh.com"));
    }

    #[test]
    fn test_sign_client_key_vault_routing_errors() {
        let (_, client_pub) = generate_client_keypair().unwrap();

        // With vault_addr set, sign_client_key should bail (no async context)
        let config = SshConfig {
            vault_addr: Some("https://vault.example.com".to_string()),
            ..SshConfig::default()
        };
        assert!(sign_client_key(&config, None, &client_pub, &["sandbox"], 1800).is_err());
    }

    #[test]
    fn test_sign_client_key_no_ca_key_errors() {
        let (_, client_pub) = generate_client_keypair().unwrap();
        let config = SshConfig::default();

        // No CA key provided for local signing should error
        assert!(sign_client_key(&config, None, &client_pub, &["sandbox"], 1800).is_err());
    }

    #[test]
    fn test_sign_client_key_with_generated_keypair() {
        // End-to-end: generate CA, generate client, sign, verify format
        let (ca_priv, _ca_pub) = generate_ca_keypair().unwrap();
        let (_, client_pub) = generate_client_keypair().unwrap();

        let cert =
            sign_client_key_local(&ca_priv, &client_pub, &["sandbox", "agent"], 600).unwrap();
        assert!(cert.contains("ssh-ed25519-cert-v01@openssh.com"));
    }
}
