//! Network-layer secret injection proxy (Gondolin pattern).
//!
//! Runs an HTTP forward proxy on the host that intercepts sandbox traffic.
//! Secrets are injected as HTTP headers for allowed hosts — they never enter the VM.
//!
//! The proxy supports:
//! - Domain allowlist enforcement (blocks unauthorized destinations)
//! - Secret header injection (Authorization, x-api-key, etc.)
//! - HTTPS MITM via per-host TLS certificates signed by a generated CA
//! - Audit logging of all proxied requests

use anyhow::{Context, Result, bail};
use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{RwLock, oneshot};

use crate::llm_intercept::{
    self, LlmDomainRegistry, LlmEvent, TokenUsage, extract_model_from_request,
    extract_streaming_from_request, extract_token_usage,
};
use crate::model_governance::{
    DenialReason, GovernanceDenialAudit, ModelGovernancePolicy, normalize_model,
    normalize_provider, record_denial,
};
use crate::proxy_hooks::{
    HookEvent, HookRegistry, ProxyEvent, dispatch_hooks, dispatch_llm_hooks, new_registry,
};
use crate::vsock_secrets::PlaceholderMap;

/// A secret binding: maps a secret key to a target host and HTTP header.
#[derive(Debug, Clone)]
pub struct SecretBinding {
    /// Key in SecretVault (e.g. "OPENAI_API_KEY")
    pub secret_key: String,
    /// Target host where this secret is injected (e.g. "api.openai.com")
    pub target_host: String,
    /// Header name to set (default: "Authorization")
    pub header_name: String,
    /// Header value prefix (default: "Bearer ")
    pub header_prefix: String,
}

impl SecretBinding {
    /// Parse a CLI binding string.
    ///
    /// Formats:
    /// - `KEY=value:host` — inline value, bind to host
    /// - `KEY:host` — lookup KEY in vault, bind to host
    /// - `KEY:host:header` — lookup KEY, bind to host, use custom header name
    ///
    /// Returns `(binding, Option<inline_value>)`.
    pub fn parse_cli(s: &str) -> Result<(Self, Option<String>)> {
        // Check for inline value: KEY=value:host
        if let Some(eq_pos) = s.find('=') {
            let key = &s[..eq_pos];
            let rest = &s[eq_pos + 1..];

            // rest is "value:host" or "value:host:header"
            let parts: Vec<&str> = rest.rsplitn(2, ':').collect();
            if parts.len() < 2 {
                bail!(
                    "Invalid secret binding '{}'. Expected KEY=value:host or KEY:host",
                    s
                );
            }
            // rsplitn reverses: parts[0] = host (or header), parts[1] = value (or value:host)
            // Actually let's parse more carefully
            let colon_positions: Vec<usize> = rest.match_indices(':').map(|(i, _)| i).collect();
            if colon_positions.is_empty() {
                bail!("Invalid secret binding '{}'. Expected KEY=value:host", s);
            }

            // Last colon separates host from value
            let last_colon = *colon_positions.last().unwrap();
            let value = &rest[..last_colon];
            let host = &rest[last_colon + 1..];

            if key.is_empty() || value.is_empty() || host.is_empty() {
                bail!(
                    "Invalid secret binding '{}'. KEY, value, and host must be non-empty",
                    s
                );
            }

            Ok((
                SecretBinding {
                    secret_key: key.to_string(),
                    target_host: host.to_string(),
                    header_name: "Authorization".to_string(),
                    header_prefix: "Bearer ".to_string(),
                },
                Some(value.to_string()),
            ))
        } else {
            // Vault lookup: KEY:host or KEY:host:header
            let parts: Vec<&str> = s.splitn(3, ':').collect();
            match parts.len() {
                2 => {
                    if parts[0].is_empty() || parts[1].is_empty() {
                        bail!(
                            "Invalid secret binding '{}'. KEY and host must be non-empty",
                            s
                        );
                    }
                    Ok((
                        SecretBinding {
                            secret_key: parts[0].to_string(),
                            target_host: parts[1].to_string(),
                            header_name: "Authorization".to_string(),
                            header_prefix: "Bearer ".to_string(),
                        },
                        None,
                    ))
                }
                3 => {
                    if parts[0].is_empty() || parts[1].is_empty() || parts[2].is_empty() {
                        bail!(
                            "Invalid secret binding '{}'. KEY, host, and header must be non-empty",
                            s
                        );
                    }
                    Ok((
                        SecretBinding {
                            secret_key: parts[0].to_string(),
                            target_host: parts[1].to_string(),
                            header_name: parts[2].to_string(),
                            header_prefix: String::new(),
                        },
                        None,
                    ))
                }
                _ => bail!(
                    "Invalid secret binding '{}'. Expected KEY:host or KEY=value:host",
                    s
                ),
            }
        }
    }
}

/// Configuration for a proxy instance.
#[derive(Debug, Clone)]
pub struct ProxyConfig {
    /// Address to listen on (e.g. 127.0.0.1:0 for auto-assign)
    pub listen_addr: SocketAddr,
    /// Secret bindings for header injection
    pub bindings: Vec<SecretBinding>,
    /// Allowed destination hosts
    pub allowed_hosts: Vec<String>,
    /// Blocked destination hosts
    pub blocked_hosts: Vec<String>,
    /// If true, only allowed_hosts can be accessed
    pub allowlist_only: bool,
    /// Sandbox name (for logging)
    pub sandbox_name: String,
    /// Hooks to dispatch on proxy events
    pub hooks: Vec<crate::proxy_hooks::ProxyHook>,
    /// Enable LLM traffic interception (default: true)
    pub llm_intercept: bool,
    /// Additional LLM domains beyond the built-in list
    pub llm_domains: Vec<String>,
    /// Domains whose keys are managed at the org level (from [llm_keys] config)
    pub org_managed_domains: Vec<String>,
    /// Resolved tenant model policy. This is populated by trusted server
    /// context; it is never selected from proxy request data.
    pub llm_governance: Option<ModelGovernancePolicy>,
}

/// Handle to a running proxy instance.
pub struct ProxyHandle {
    /// Actual address the proxy is listening on
    pub addr: SocketAddr,
    /// PEM-encoded CA certificate (for injection into sandbox trust store)
    pub ca_cert_pem: String,
    /// Shutdown signal
    pub shutdown_tx: oneshot::Sender<()>,
    /// Hook registry for managing hooks at runtime
    pub hook_registry: HookRegistry,
}

/// Shared state for the proxy.
struct ProxyState {
    config: ProxyConfig,
    /// Resolved secrets: host -> (header_name, header_value)
    resolved_secrets: HashMap<String, (String, String)>,
    /// Placeholder token → real value map for body/header substitution
    placeholder_map: PlaceholderMap,
    /// CA signing context (cert + key pair) for generating per-host certs
    ca_signer: Arc<CaSigner>,
    /// Cached per-host TLS server configs (for MITM)
    cert_cache: HashMap<String, Arc<rustls::ServerConfig>>,
    /// Hook registry for dispatching proxy events
    hook_registry: HookRegistry,
    /// LLM domain registry for traffic interception
    llm_registry: LlmDomainRegistry,
    /// Domains whose keys are managed at the org level
    org_managed_domains: std::collections::HashSet<String>,
}

/// Holds the CA cert and key pair for signing per-host certs.
pub struct CaSigner {
    issuer: rcgen::CertifiedIssuer<'static, rcgen::KeyPair>,
}

/// Generate a CA certificate and key pair for the proxy.
///
/// Returns `(ca_cert_pem, CaSigner)`. The PEM is for injection into sandbox trust stores.
pub fn generate_proxy_ca() -> Result<(String, CaSigner)> {
    use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyUsagePurpose};

    let mut params =
        CertificateParams::new(Vec::<String>::new()).context("Failed to create CA params")?;
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "AgentKernel Proxy CA");
    params
        .distinguished_name
        .push(rcgen::DnType::OrganizationName, "AgentKernel");

    let key_pair = rcgen::KeyPair::generate().context("Failed to generate CA key pair")?;
    let ca = rcgen::CertifiedIssuer::self_signed(params, key_pair)
        .context("Failed to self-sign CA certificate")?;

    let cert_pem = ca.pem();

    Ok((cert_pem, CaSigner { issuer: ca }))
}

/// Generate a per-host TLS certificate signed by the proxy CA.
///
/// Returns a rustls `ServerConfig` ready to accept TLS connections for this host.
fn generate_host_tls_config(host: &str, ca: &CaSigner) -> Result<Arc<rustls::ServerConfig>> {
    use rcgen::{CertificateParams, IsCa, KeyUsagePurpose};
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};

    let host_only = host.split(':').next().unwrap_or(host);
    let mut params = CertificateParams::new(vec![host_only.to_string()])
        .context("Failed to create host cert params")?;
    params.is_ca = IsCa::NoCa;
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, host_only);

    let host_key = rcgen::KeyPair::generate().context("Failed to generate host key")?;
    let host_cert = params
        .signed_by(&host_key, &ca.issuer)
        .context("Failed to sign host cert")?;

    let cert_chain = vec![CertificateDer::from(host_cert.der().to_vec())];
    let key_der = PrivateKeyDer::try_from(host_key.serialize_der())
        .map_err(|e| anyhow::anyhow!("Failed to convert host key: {}", e))?;

    let tls_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, key_der)
        .context("Failed to build TLS config")?;

    Ok(Arc::new(tls_config))
}

/// Check if a host is allowed by the proxy's domain policy.
pub fn is_host_allowed(host: &str, config: &ProxyConfig) -> bool {
    // Strip port if present
    let host_only = host.split(':').next().unwrap_or(host);

    // Check blocklist first
    for blocked in &config.blocked_hosts {
        if matches_domain(host_only, blocked) {
            return false;
        }
    }

    // If allowlist_only, host must be in allowed_hosts
    if config.allowlist_only {
        return config
            .allowed_hosts
            .iter()
            .any(|a| matches_domain(host_only, a));
    }

    // Otherwise, allow unless blocked
    true
}

/// Check if a hostname matches a domain pattern (supports *.example.com wildcards).
fn matches_domain(host: &str, pattern: &str) -> bool {
    if pattern.starts_with("*.") {
        let suffix = &pattern[1..]; // ".example.com"
        host.ends_with(suffix) || host == &pattern[2..]
    } else {
        host == pattern
    }
}

/// Start the proxy server.
///
/// `resolved_secrets` maps host -> (header_name, header_value).
/// `placeholder_map` maps placeholder tokens to real secret values for body substitution.
/// The caller is responsible for resolving secrets from the vault.
pub async fn start_proxy(
    config: ProxyConfig,
    resolved_secrets: HashMap<String, (String, String)>,
    placeholder_map: PlaceholderMap,
) -> Result<ProxyHandle> {
    // Ensure rustls crypto provider is installed (needed for TLS MITM)
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Generate CA for MITM
    let (ca_cert_pem, ca_signer) = generate_proxy_ca()?;

    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    let hook_registry = new_registry(config.hooks.clone());

    // Governance requires the MITM/interceptor even when a caller supplied a
    // stale `llm_intercept = false` setting. Otherwise CONNECT would tunnel
    // straight through and bypass the allowlist.
    let llm_intercept =
        llm_interception_enabled(config.llm_intercept, config.llm_governance.is_some());
    let llm_registry = if llm_intercept {
        LlmDomainRegistry::default_registry().with_custom_domains(&config.llm_domains)
    } else {
        LlmDomainRegistry::empty()
    };

    if let Some(policy) = config.llm_governance.as_ref() {
        let registered = config
            .llm_domains
            .iter()
            .map(|_| "custom")
            .chain([
                "openai",
                "anthropic",
                "google",
                "deepseek",
                "groq",
                "mistral",
                "cohere",
                "together",
                "fireworks",
            ])
            .collect::<std::collections::HashSet<_>>();
        if policy
            .providers()
            .any(|provider| !registered.contains(provider))
        {
            bail!("LLM governance policy references a provider with no trusted proxy domain")
        }
    }

    let listener = TcpListener::bind(&config.listen_addr)
        .await
        .context("Failed to bind proxy listener")?;
    let actual_addr = listener.local_addr()?;

    let org_managed_domains: std::collections::HashSet<String> =
        config.org_managed_domains.iter().cloned().collect();

    let state = Arc::new(RwLock::new(ProxyState {
        config,
        resolved_secrets,
        placeholder_map,
        ca_signer: Arc::new(ca_signer),
        cert_cache: HashMap::new(),
        hook_registry: hook_registry.clone(),
        llm_registry,
        org_managed_domains,
    }));

    // Spawn the proxy accept loop
    let _handle = tokio::spawn(run_proxy(listener, state, shutdown_rx));

    eprintln!("[proxy] Listening on {}", actual_addr);

    Ok(ProxyHandle {
        addr: actual_addr,
        ca_cert_pem,
        shutdown_tx,
        hook_registry,
    })
}

async fn run_proxy(
    listener: TcpListener,
    state: Arc<RwLock<ProxyState>>,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    loop {
        tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok((stream, _addr)) => {
                        let state = state.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle_connection(stream, state).await {
                                eprintln!("[proxy] Connection error: {}", e);
                            }
                        });
                    }
                    Err(e) => {
                        eprintln!("[proxy] Accept error: {}", e);
                    }
                }
            }
            _ = &mut shutdown_rx => {
                eprintln!("[proxy] Shutting down");
                break;
            }
        }
    }
}

async fn handle_connection(stream: TcpStream, state: Arc<RwLock<ProxyState>>) -> Result<()> {
    let io = TokioIo::new(stream);

    http1::Builder::new()
        .preserve_header_case(true)
        .title_case_headers(true)
        .serve_connection(
            io,
            service_fn(move |req| {
                let state = state.clone();
                async move { handle_request(req, state).await }
            }),
        )
        .with_upgrades()
        .await
        .context("HTTP connection error")?;

    Ok(())
}

type BoxBody = http_body_util::combinators::BoxBody<bytes::Bytes, hyper::Error>;
const MAX_GOVERNED_REQUEST_BODY_BYTES: usize = 8 * 1024 * 1024;

fn llm_interception_enabled(configured: bool, governance: bool) -> bool {
    configured || governance
}

fn empty_body() -> BoxBody {
    use http_body_util::Empty;
    Empty::new().map_err(|never| match never {}).boxed()
}

fn full_body(s: impl Into<bytes::Bytes>) -> BoxBody {
    use http_body_util::Full;
    Full::new(s.into()).map_err(|never| match never {}).boxed()
}

/// Return a generic denial response and emit a metadata-only audit record.
/// Request bodies, header values, and unsanitized model input are intentionally
/// excluded from both the log and the in-memory audit trail.
#[allow(clippy::too_many_arguments)]
async fn governance_denial(
    policy: &ModelGovernancePolicy,
    reason: DenialReason,
    sandbox: &str,
    provider: &str,
    host: &str,
    method: &str,
    path: &str,
    model: Option<&str>,
) -> Response<BoxBody> {
    let provider = normalize_provider(provider).unwrap_or_else(|_| "unknown".to_string());
    let model = model.and_then(|value| normalize_model(value).ok());
    eprintln!(
        "[proxy:audit] LLM governance denied sandbox={} tenant={} provider={} model={} host={} method={} path={} reason={}",
        sandbox,
        policy.tenant_id(),
        provider,
        model.as_deref().unwrap_or("unknown"),
        host,
        method,
        path,
        reason.as_str(),
    );
    crate::audit::log_event(crate::audit::AuditEvent::PolicyViolation {
        sandbox: sandbox.to_string(),
        policy: "llm_model_governance".to_string(),
        details: format!(
            "tenant={} provider={} model={} host={} method={} path={} reason={}",
            policy.tenant_id(),
            provider,
            model.as_deref().unwrap_or("unknown"),
            host,
            method,
            path,
            reason.as_str(),
        ),
    });
    record_denial(GovernanceDenialAudit {
        timestamp: chrono::Utc::now().to_rfc3339(),
        sandbox: sandbox.to_string(),
        tenant: policy.tenant_id().to_string(),
        provider: provider.clone(),
        host: host.to_string(),
        method: method.to_string(),
        path: path.to_string(),
        model,
        reason,
    })
    .await;

    let mut response = Response::new(full_body("LLM model request denied by governance policy"));
    *response.status_mut() = StatusCode::FORBIDDEN;
    response
}

async fn handle_request(
    req: Request<Incoming>,
    state: Arc<RwLock<ProxyState>>,
) -> Result<Response<BoxBody>, hyper::Error> {
    if req.method() == Method::CONNECT {
        handle_connect(req, state).await
    } else {
        handle_plain_http(req, state).await
    }
}

/// Handle HTTPS CONNECT with MITM for secret header injection.
///
/// For hosts with secret bindings: terminates TLS from client using a cert
/// signed by proxy CA, connects upstream with real TLS, bridges HTTP with
/// header injection.
///
/// For hosts without bindings: plain tunnel (passthrough) — no MITM needed.
async fn handle_connect(
    req: Request<Incoming>,
    state: Arc<RwLock<ProxyState>>,
) -> Result<Response<BoxBody>, hyper::Error> {
    let host = req
        .uri()
        .authority()
        .map(|a| a.to_string())
        .unwrap_or_default();

    let host_only = host.split(':').next().unwrap_or(&host).to_string();

    let s = state.read().await;
    if !is_host_allowed(&host, &s.config) {
        eprintln!(
            "[proxy] BLOCKED CONNECT to {} (sandbox: {})",
            host, s.config.sandbox_name
        );
        let mut resp = Response::new(full_body("Forbidden: host not in allowlist"));
        *resp.status_mut() = StatusCode::FORBIDDEN;
        return Ok(resp);
    }

    let has_secret = s.resolved_secrets.contains_key(&host_only);
    let secret_info = s.resolved_secrets.get(&host_only).cloned();
    let is_llm_host =
        llm_interception_enabled(s.config.llm_intercept, s.config.llm_governance.is_some())
            && s.llm_registry.lookup(&host_only).is_some();
    let llm_provider = s.llm_registry.lookup(&host_only).cloned();
    let llm_governance = s.config.llm_governance.clone();
    let ca_signer = s.ca_signer.clone();
    let sandbox_name = s.config.sandbox_name.clone();
    let registry = s.hook_registry.clone();
    let is_org_managed = s.org_managed_domains.contains(&host_only);
    let placeholder_map = s.placeholder_map.clone();
    drop(s);

    // Dispatch OnRequest hook for CONNECT
    dispatch_hooks(
        &HookEvent::OnRequest,
        &ProxyEvent {
            timestamp: chrono::Utc::now().to_rfc3339(),
            sandbox: sandbox_name.clone(),
            method: "CONNECT".to_string(),
            url: host.clone(),
            host: host_only.clone(),
            status: None,
            secret_injected: has_secret,
            latency_ms: None,
        },
        &registry,
    )
    .await;

    let addr = if host.contains(':') {
        host.clone()
    } else {
        format!("{}:443", host)
    };

    // MITM if we have a secret binding OR this is a known LLM host
    let needs_mitm = secret_info.is_some() || is_llm_host;

    if !needs_mitm {
        // Plain tunnel (passthrough) — no MITM overhead
        tokio::task::spawn(async move {
            match hyper::upgrade::on(req).await {
                Ok(upgraded) => {
                    if let Ok(upstream) = TcpStream::connect(&addr).await {
                        let (mut cr, mut cw) = tokio::io::split(TokioIo::new(upgraded));
                        let (mut ur, mut uw) = tokio::io::split(upstream);
                        let _ = tokio::try_join!(
                            tokio::io::copy(&mut cr, &mut uw),
                            tokio::io::copy(&mut ur, &mut cw)
                        );
                    }
                }
                Err(e) => eprintln!("[proxy] Upgrade error: {}", e),
            }
        });
        return Ok(Response::new(empty_body()));
    }

    // MITM path: terminate TLS from client, inject headers, forward to upstream
    // For LLM-only MITM (no secret), use empty header injection
    let state_for_mitm = state.clone();
    tokio::task::spawn(async move {
        let (header_name, header_value) = secret_info.unwrap_or_default();

        let upgraded = match hyper::upgrade::on(req).await {
            Ok(u) => u,
            Err(e) => {
                eprintln!("[proxy] Upgrade error: {}", e);
                return;
            }
        };

        // Get or generate TLS config for this host
        let tls_config = {
            let mut s = state_for_mitm.write().await;
            if let Some(cfg) = s.cert_cache.get(&host_only) {
                cfg.clone()
            } else {
                match generate_host_tls_config(&host_only, &ca_signer) {
                    Ok(cfg) => {
                        s.cert_cache.insert(host_only.clone(), cfg.clone());
                        cfg
                    }
                    Err(e) => {
                        eprintln!(
                            "[proxy] Failed to generate host cert for {}: {}",
                            host_only, e
                        );
                        return;
                    }
                }
            }
        };

        // TLS-accept the client connection using our MITM cert
        let tls_acceptor = tokio_rustls::TlsAcceptor::from(tls_config);
        let client_tls = match tls_acceptor.accept(TokioIo::new(upgraded)).await {
            Ok(stream) => stream,
            Err(e) => {
                eprintln!("[proxy] TLS accept error for {}: {}", host_only, e);
                return;
            }
        };

        // Bridge: read HTTP from client TLS, inject header, forward to upstream HTTPS
        if let Err(e) = mitm_bridge(
            client_tls,
            &addr,
            &host_only,
            &header_name,
            &header_value,
            &sandbox_name,
            llm_provider.as_ref(),
            llm_governance.as_ref(),
            &registry,
            is_org_managed,
            &placeholder_map,
        )
        .await
        {
            eprintln!("[proxy] MITM bridge error for {}: {}", host_only, e);
        }
    });

    Ok(Response::new(empty_body()))
}

/// Bridge between client (TLS-terminated) and upstream (TLS), injecting secret headers.
/// When `llm_provider` is Some, also captures LLM request/response metadata.
/// When `placeholder_map` is non-empty, scans request bodies for placeholder tokens
/// and substitutes real secret values before forwarding upstream.
#[allow(clippy::too_many_arguments)]
async fn mitm_bridge(
    client_tls: tokio_rustls::server::TlsStream<TokioIo<hyper::upgrade::Upgraded>>,
    upstream_addr: &str,
    host: &str,
    header_name: &str,
    header_value: &str,
    sandbox_name: &str,
    llm_provider: Option<&llm_intercept::LlmProvider>,
    llm_governance: Option<&ModelGovernancePolicy>,
    hook_registry: &HookRegistry,
    is_org_managed: bool,
    placeholder_map: &PlaceholderMap,
) -> Result<()> {
    // Connect to upstream with real TLS
    let upstream_tcp = TcpStream::connect(upstream_addr)
        .await
        .with_context(|| format!("Failed to connect to upstream {}", upstream_addr))?;

    let mut root_store = rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let upstream_tls_config = Arc::new(
        rustls::ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth(),
    );

    let server_name = rustls::pki_types::ServerName::try_from(host.to_string())
        .map_err(|e| anyhow::anyhow!("Invalid server name '{}': {}", host, e))?;
    let connector = tokio_rustls::TlsConnector::from(upstream_tls_config);
    let upstream_tls = connector
        .connect(server_name, upstream_tcp)
        .await
        .with_context(|| format!("TLS connect to {} failed", host))?;

    // HTTP bridge: read requests from client, inject header, send to upstream
    let client_io = TokioIo::new(client_tls);
    let upstream_io = TokioIo::new(upstream_tls);

    let header_name = header_name.to_string();
    let header_value = header_value.to_string();
    let sandbox_name = sandbox_name.to_string();
    let host = host.to_string();
    let llm_provider_name = llm_provider.map(|p| p.name.to_string());
    let llm_token_format = llm_provider.map(|p| p.token_format);
    let llm_governance = llm_governance.cloned();
    let hook_registry = hook_registry.clone();
    let placeholder_map = placeholder_map.clone();

    // Use hyper to serve the client side and forward to upstream
    let (upstream_sender, upstream_conn) = hyper::client::conn::http1::handshake(upstream_io)
        .await
        .context("Upstream HTTP handshake failed")?;

    tokio::spawn(async move {
        if let Err(e) = upstream_conn.await {
            eprintln!("[proxy] Upstream connection error: {}", e);
        }
    });

    let sender = Arc::new(tokio::sync::Mutex::new(upstream_sender));

    http1::Builder::new()
        .preserve_header_case(true)
        .title_case_headers(true)
        .serve_connection(
            client_io,
            service_fn(move |req: Request<Incoming>| {
                let hn = header_name.clone();
                let hv = header_value.clone();
                let sn = sandbox_name.clone();
                let host = host.clone();
                let sender = sender.clone();
                let llm_provider_name = llm_provider_name.clone();
                let llm_token_format = llm_token_format;
                let hook_registry = hook_registry.clone();
                let pmap = placeholder_map.clone();
                let llm_governance = llm_governance.clone();
                async move {
                    let method_str = req.method().to_string();
                    let uri_path = req.uri().path().to_string();
                    let has_secret = !hn.is_empty();

                    eprintln!(
                        "[proxy] MITM {} {} (sandbox: {}, secret: {})",
                        method_str, uri_path, sn, has_secret
                    );

                    // For LLM requests, buffer the request body for metadata extraction
                    let mut model_name = None;
                    let has_placeholders = !pmap.is_empty();
                    let req = if llm_provider_name.is_some() || has_placeholders {
                        // Buffer body for LLM metadata extraction and/or placeholder substitution
                        let (parts, body) = req.into_parts();
                        let mut body_bytes = match body.collect().await {
                            Ok(collected) => collected.to_bytes(),
                            Err(_) if llm_governance.is_some() && llm_provider_name.is_some() => {
                                let policy = llm_governance.as_ref().expect("checked above");
                                let provider = llm_provider_name.as_deref().unwrap_or("unknown");
                                return Ok::<_, hyper::Error>(
                                    governance_denial(
                                        policy,
                                        DenialReason::InvalidModel,
                                        &sn,
                                        provider,
                                        &host,
                                        &method_str,
                                        &uri_path,
                                        None,
                                    )
                                    .await,
                                );
                            }
                            Err(_) => bytes::Bytes::new(),
                        };

                        if let Some(policy) = llm_governance.as_ref()
                            && llm_provider_name.is_some()
                            && body_bytes.len() > MAX_GOVERNED_REQUEST_BODY_BYTES
                        {
                            let provider = llm_provider_name.as_deref().unwrap_or("unknown");
                            return Ok::<_, hyper::Error>(
                                governance_denial(
                                    policy,
                                    DenialReason::InvalidModel,
                                    &sn,
                                    provider,
                                    &host,
                                    &method_str,
                                    &uri_path,
                                    None,
                                )
                                .await,
                            );
                        }

                        // Substitute placeholder tokens with real secret values
                        if has_placeholders {
                            let (substituted, replaced) = pmap.substitute_bytes(&body_bytes);
                            if replaced {
                                body_bytes = bytes::Bytes::from(substituted);
                            }
                        }

                        model_name = extract_model_from_request(&body_bytes);
                        let is_str = extract_streaming_from_request(&body_bytes);
                        if let (Some(policy), Some(provider)) =
                            (llm_governance.as_ref(), llm_provider_name.as_deref())
                            && let Err(reason) = policy.authorize(provider, model_name.as_deref())
                        {
                            return Ok::<_, hyper::Error>(
                                governance_denial(
                                    policy,
                                    reason,
                                    &sn,
                                    provider,
                                    &host,
                                    &method_str,
                                    &uri_path,
                                    model_name.as_deref(),
                                )
                                .await,
                            );
                        }
                        let mut new_req = Request::from_parts(
                            parts,
                            http_body_util::Full::new(body_bytes)
                                .map_err(|never| match never {})
                                .boxed(),
                        );
                        if has_secret
                            && let Ok(val) = hyper::header::HeaderValue::from_str(&hv)
                            && let Ok(name) = hyper::header::HeaderName::from_bytes(hn.as_bytes())
                        {
                            new_req.headers_mut().insert(name, val);
                        }
                        // Substitute placeholders in header values
                        if has_placeholders {
                            substitute_header_placeholders(new_req.headers_mut(), &pmap);
                        }
                        (new_req, is_str)
                    } else {
                        // Non-LLM, no placeholders: inject header directly
                        let (parts, body) = req.into_parts();
                        let mut new_req = Request::from_parts(parts, body.boxed());
                        if has_secret
                            && let Ok(val) = hyper::header::HeaderValue::from_str(&hv)
                            && let Ok(name) = hyper::header::HeaderName::from_bytes(hn.as_bytes())
                        {
                            new_req.headers_mut().insert(name, val);
                        }
                        (new_req, false)
                    };
                    let (req, is_streaming) = req;

                    let start = std::time::Instant::now();
                    let mut upstream_sender = sender.lock().await;
                    match upstream_sender.send_request(req).await {
                        Ok(resp) => {
                            let status = resp.status().as_u16();
                            let latency = start.elapsed().as_millis() as u64;

                            if let Some(ref provider) = llm_provider_name {
                                if is_streaming {
                                    // Streaming: pass through, emit event without token counts
                                    let llm_event = LlmEvent {
                                        timestamp: chrono::Utc::now().to_rfc3339(),
                                        sandbox: sn,
                                        provider: provider.clone(),
                                        host: host.clone(),
                                        method: method_str,
                                        path: uri_path,
                                        model: model_name,
                                        status: Some(status),
                                        latency_ms: Some(latency),
                                        input_tokens: None,
                                        output_tokens: None,
                                        total_tokens: None,
                                        streaming: true,
                                        secret_injected: has_secret,
                                        key_source: if is_org_managed {
                                            "org".to_string()
                                        } else if has_secret {
                                            "sandbox".to_string()
                                        } else {
                                            "none".to_string()
                                        },
                                        tenant: None,
                                        agent: None,
                                        user: None,
                                        project: None,
                                    };
                                    llm_intercept::record_llm_event(&llm_event).await;
                                    crate::metrics::record_llm_request(
                                        &llm_event.provider,
                                        llm_event.model.as_deref().unwrap_or("unknown"),
                                        0,
                                        0,
                                    );
                                    dispatch_llm_hooks(&llm_event, &hook_registry).await;

                                    let (parts, body) = resp.into_parts();
                                    Ok::<_, hyper::Error>(Response::from_parts(parts, body.boxed()))
                                } else {
                                    // Non-streaming: buffer response, extract token usage
                                    let (parts, body) = resp.into_parts();
                                    let resp_bytes = body
                                        .collect()
                                        .await
                                        .map(|c| c.to_bytes())
                                        .unwrap_or_default();

                                    let usage = if let Some(fmt) = llm_token_format {
                                        extract_token_usage(&resp_bytes, &fmt)
                                    } else {
                                        TokenUsage::default()
                                    };

                                    let llm_event = LlmEvent {
                                        timestamp: chrono::Utc::now().to_rfc3339(),
                                        sandbox: sn,
                                        provider: provider.clone(),
                                        host: host.clone(),
                                        method: method_str,
                                        path: uri_path,
                                        model: model_name,
                                        status: Some(status),
                                        latency_ms: Some(latency),
                                        input_tokens: usage.input_tokens,
                                        output_tokens: usage.output_tokens,
                                        total_tokens: usage.total_tokens,
                                        streaming: false,
                                        secret_injected: has_secret,
                                        key_source: if is_org_managed {
                                            "org".to_string()
                                        } else if has_secret {
                                            "sandbox".to_string()
                                        } else {
                                            "none".to_string()
                                        },
                                        tenant: None,
                                        agent: None,
                                        user: None,
                                        project: None,
                                    };
                                    llm_intercept::record_llm_event(&llm_event).await;
                                    crate::metrics::record_llm_request(
                                        &llm_event.provider,
                                        llm_event.model.as_deref().unwrap_or("unknown"),
                                        llm_event.input_tokens.unwrap_or(0),
                                        llm_event.output_tokens.unwrap_or(0),
                                    );
                                    dispatch_llm_hooks(&llm_event, &hook_registry).await;

                                    Ok(Response::from_parts(parts, full_body(resp_bytes)))
                                }
                            } else {
                                // Non-LLM: pass through
                                let (parts, body) = resp.into_parts();
                                Ok(Response::from_parts(parts, body.boxed()))
                            }
                        }
                        Err(e) => {
                            eprintln!("[proxy] MITM upstream error: {}", e);
                            let mut resp = Response::new(full_body(format!("Proxy error: {}", e)));
                            *resp.status_mut() = StatusCode::BAD_GATEWAY;
                            Ok(resp)
                        }
                    }
                }
            }),
        )
        .await
        .context("MITM HTTP server error")?;

    Ok(())
}

/// Handle plain HTTP requests with header injection.
async fn handle_plain_http(
    mut req: Request<Incoming>,
    state: Arc<RwLock<ProxyState>>,
) -> Result<Response<BoxBody>, hyper::Error> {
    let host = req
        .uri()
        .host()
        .or_else(|| {
            req.headers()
                .get("host")
                .and_then(|h| h.to_str().ok())
                .map(|h| h.split(':').next().unwrap_or(h))
        })
        .unwrap_or("")
        .to_string();

    let s = state.read().await;
    if !is_host_allowed(&host, &s.config) {
        eprintln!(
            "[proxy] BLOCKED HTTP {} {} (sandbox: {})",
            req.method(),
            req.uri(),
            s.config.sandbox_name
        );
        let mut resp = Response::new(full_body("Forbidden: host not in allowlist"));
        *resp.status_mut() = StatusCode::FORBIDDEN;
        return Ok(resp);
    }

    // Inject secret header if we have a binding for this host
    let mut secret_injected = false;
    if let Some((header_name, header_value)) = s.resolved_secrets.get(&host)
        && let Ok(value) = hyper::header::HeaderValue::from_str(header_value)
    {
        req.headers_mut().insert(
            hyper::header::HeaderName::from_bytes(header_name.as_bytes())
                .unwrap_or(hyper::header::AUTHORIZATION),
            value,
        );
        secret_injected = true;
    }

    // Substitute placeholder tokens in headers
    let has_placeholders = !s.placeholder_map.is_empty();
    let placeholder_map = s.placeholder_map.clone();
    if has_placeholders {
        substitute_header_placeholders(req.headers_mut(), &placeholder_map);
    }

    let sandbox_name = s.config.sandbox_name.clone();
    let method_str = req.method().to_string();
    let url_str = req.uri().to_string();
    let uri_path = req.uri().path().to_string();
    let registry = s.hook_registry.clone();
    let llm_provider =
        if llm_interception_enabled(s.config.llm_intercept, s.config.llm_governance.is_some()) {
            s.llm_registry.lookup(&host).cloned()
        } else {
            None
        };
    let llm_governance = s.config.llm_governance.clone();
    let is_org_managed_http = s.org_managed_domains.contains(&host);
    drop(s);

    // Dispatch OnRequest hook
    let start = std::time::Instant::now();
    dispatch_hooks(
        &HookEvent::OnRequest,
        &ProxyEvent {
            timestamp: chrono::Utc::now().to_rfc3339(),
            sandbox: sandbox_name.clone(),
            method: method_str.clone(),
            url: url_str.clone(),
            host: host.clone(),
            status: None,
            secret_injected,
            latency_ms: None,
        },
        &registry,
    )
    .await;

    // Buffer LLM requests so model governance runs before any upstream send.
    let mut model_name = None;
    let mut is_streaming = false;
    let req = if has_placeholders || llm_provider.is_some() {
        let (parts, body) = req.into_parts();
        let mut body_bytes = match body.collect().await {
            Ok(collected) => collected.to_bytes(),
            Err(_) if llm_governance.is_some() && llm_provider.is_some() => {
                let policy = llm_governance.as_ref().expect("checked above");
                let provider = llm_provider
                    .as_ref()
                    .map(|provider| provider.name)
                    .unwrap_or("unknown");
                return Ok(governance_denial(
                    policy,
                    DenialReason::InvalidModel,
                    &sandbox_name,
                    provider,
                    &host,
                    &method_str,
                    &uri_path,
                    None,
                )
                .await);
            }
            Err(_) => bytes::Bytes::new(),
        };
        if let Some(policy) = llm_governance.as_ref()
            && llm_provider.is_some()
            && body_bytes.len() > MAX_GOVERNED_REQUEST_BODY_BYTES
        {
            let provider = llm_provider
                .as_ref()
                .map(|provider| provider.name)
                .unwrap_or("unknown");
            return Ok(governance_denial(
                policy,
                DenialReason::InvalidModel,
                &sandbox_name,
                provider,
                &host,
                &method_str,
                &uri_path,
                None,
            )
            .await);
        }
        model_name = extract_model_from_request(&body_bytes);
        is_streaming = extract_streaming_from_request(&body_bytes);
        if let Some(policy) = llm_governance.as_ref()
            && let Some(provider) = llm_provider.as_ref()
            && let Err(reason) = policy.authorize(provider.name, model_name.as_deref())
        {
            return Ok(governance_denial(
                policy,
                reason,
                &sandbox_name,
                provider.name,
                &host,
                &method_str,
                &uri_path,
                model_name.as_deref(),
            )
            .await);
        }
        if has_placeholders {
            let (substituted, _) = placeholder_map.substitute_bytes(&body_bytes);
            body_bytes = bytes::Bytes::from(substituted);
        }
        Request::from_parts(
            parts,
            http_body_util::Full::new(body_bytes)
                .map_err(|never| match never {})
                .boxed(),
        )
    } else {
        let (parts, body) = req.into_parts();
        Request::from_parts(parts, body.boxed())
    };

    // Forward the request to the upstream server
    match forward_boxed_request(req).await {
        Ok(resp) => {
            let latency = start.elapsed().as_millis() as u64;
            let status = resp.status().as_u16();
            dispatch_hooks(
                &HookEvent::OnResponse,
                &ProxyEvent {
                    timestamp: chrono::Utc::now().to_rfc3339(),
                    sandbox: sandbox_name.clone(),
                    method: method_str.clone(),
                    url: url_str,
                    host: host.clone(),
                    status: Some(status),
                    secret_injected,
                    latency_ms: Some(latency),
                },
                &registry,
            )
            .await;

            // LLM interception for plain HTTP (rare but handles testing)
            if let Some(provider) = llm_provider {
                let (parts, body) = resp.into_parts();
                let resp_bytes = body
                    .collect()
                    .await
                    .map(|c| c.to_bytes())
                    .unwrap_or_default();
                let usage = extract_token_usage(&resp_bytes, &provider.token_format);
                let llm_event = LlmEvent {
                    timestamp: chrono::Utc::now().to_rfc3339(),
                    sandbox: sandbox_name,
                    provider: provider.name.to_string(),
                    host,
                    method: method_str,
                    path: uri_path,
                    model: model_name,
                    status: Some(status),
                    latency_ms: Some(latency),
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                    total_tokens: usage.total_tokens,
                    streaming: is_streaming,
                    secret_injected,
                    key_source: if is_org_managed_http {
                        "org".to_string()
                    } else if secret_injected {
                        "sandbox".to_string()
                    } else {
                        "none".to_string()
                    },
                    tenant: None,
                    agent: None,
                    user: None,
                    project: None,
                };
                llm_intercept::record_llm_event(&llm_event).await;
                crate::metrics::record_llm_request(
                    &llm_event.provider,
                    llm_event.model.as_deref().unwrap_or("unknown"),
                    llm_event.input_tokens.unwrap_or(0),
                    llm_event.output_tokens.unwrap_or(0),
                );
                dispatch_llm_hooks(&llm_event, &registry).await;
                Ok(Response::from_parts(parts, full_body(resp_bytes)))
            } else {
                Ok(resp)
            }
        }
        Err(e) => {
            eprintln!("[proxy] Forward error: {}", e);
            let mut resp = Response::new(full_body(format!("Proxy error: {}", e)));
            *resp.status_mut() = StatusCode::BAD_GATEWAY;
            Ok(resp)
        }
    }
}

/// Substitute placeholder tokens in HTTP header values with real secret values.
fn substitute_header_placeholders(
    headers: &mut hyper::header::HeaderMap,
    placeholder_map: &PlaceholderMap,
) {
    let mut replacements = Vec::new();
    for (name, value) in headers.iter() {
        if let Ok(val_str) = value.to_str() {
            let (substituted, replaced) = placeholder_map.substitute(val_str);
            if replaced && let Ok(new_val) = hyper::header::HeaderValue::from_str(&substituted) {
                replacements.push((name.clone(), new_val));
            }
        }
    }
    for (name, value) in replacements {
        headers.insert(name, value);
    }
}

/// Forward an HTTP request with a boxed body to the upstream server.
/// Used when the request body has been buffered for placeholder substitution.
async fn forward_boxed_request(req: Request<BoxBody>) -> Result<Response<BoxBody>> {
    use http_body_util::BodyExt;

    let uri = req.uri().clone();
    let host = uri.host().context("No host in request URI")?;
    let port = uri.port_u16().unwrap_or(80);
    let addr = format!("{}:{}", host, port);

    let stream = TcpStream::connect(&addr)
        .await
        .with_context(|| format!("Failed to connect to {}", addr))?;

    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io)
        .await
        .context("HTTP handshake failed")?;

    tokio::spawn(async move {
        if let Err(e) = conn.await {
            eprintln!("[proxy] Connection task error: {}", e);
        }
    });

    let resp = sender
        .send_request(req)
        .await
        .context("Failed to send request")?;

    let (parts, body) = resp.into_parts();
    let body = body.boxed();
    Ok(Response::from_parts(parts, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn governance_forces_llm_interception_even_when_disabled_by_stale_config() {
        assert!(llm_interception_enabled(false, true));
        assert!(!llm_interception_enabled(false, false));
    }

    #[tokio::test]
    async fn governance_rejects_unregistered_provider_at_proxy_startup() {
        let mut providers = std::collections::BTreeMap::new();
        providers.insert(
            "unregistered-provider".to_string(),
            ["model".to_string()].into_iter().collect(),
        );
        let mut tenants = std::collections::BTreeMap::new();
        tenants.insert("acme".to_string(), providers);
        let governance = ModelGovernancePolicy::from_config(
            &crate::config::LlmGovernanceConfig {
                enabled: true,
                tenants,
            },
            "acme",
        )
        .unwrap()
        .unwrap();

        let result = start_proxy(
            ProxyConfig {
                listen_addr: "127.0.0.1:0".parse().unwrap(),
                bindings: vec![],
                allowed_hosts: vec![],
                blocked_hosts: vec![],
                allowlist_only: false,
                sandbox_name: "test".to_string(),
                hooks: vec![],
                llm_intercept: false,
                llm_domains: vec![],
                org_managed_domains: vec![],
                llm_governance: Some(governance),
            },
            HashMap::new(),
            PlaceholderMap::new(),
        )
        .await;
        assert!(result.is_err());
        assert!(
            result
                .err()
                .is_some_and(|error| error.to_string().contains("no trusted proxy domain"))
        );
    }

    #[test]
    fn test_parse_cli_inline_value() {
        let (binding, value) =
            SecretBinding::parse_cli("OPENAI_API_KEY=sk-test123:api.openai.com").unwrap();
        assert_eq!(binding.secret_key, "OPENAI_API_KEY");
        assert_eq!(binding.target_host, "api.openai.com");
        assert_eq!(binding.header_name, "Authorization");
        assert_eq!(binding.header_prefix, "Bearer ");
        assert_eq!(value, Some("sk-test123".to_string()));
    }

    #[test]
    fn test_parse_cli_vault_lookup() {
        let (binding, value) = SecretBinding::parse_cli("OPENAI_API_KEY:api.openai.com").unwrap();
        assert_eq!(binding.secret_key, "OPENAI_API_KEY");
        assert_eq!(binding.target_host, "api.openai.com");
        assert_eq!(binding.header_name, "Authorization");
        assert_eq!(binding.header_prefix, "Bearer ");
        assert_eq!(value, None);
    }

    #[test]
    fn test_parse_cli_custom_header() {
        let (binding, value) =
            SecretBinding::parse_cli("ANTHROPIC_API_KEY:api.anthropic.com:x-api-key").unwrap();
        assert_eq!(binding.secret_key, "ANTHROPIC_API_KEY");
        assert_eq!(binding.target_host, "api.anthropic.com");
        assert_eq!(binding.header_name, "x-api-key");
        assert_eq!(binding.header_prefix, "");
        assert_eq!(value, None);
    }

    #[test]
    fn test_parse_cli_inline_value_with_colons() {
        // Value contains colons (like a base64 key)
        let (binding, value) = SecretBinding::parse_cli("MY_KEY=abc:def:ghi:my-host.com").unwrap();
        assert_eq!(binding.secret_key, "MY_KEY");
        assert_eq!(binding.target_host, "my-host.com");
        assert_eq!(value, Some("abc:def:ghi".to_string()));
    }

    #[test]
    fn test_parse_cli_invalid() {
        assert!(SecretBinding::parse_cli("NOHOST").is_err());
        assert!(SecretBinding::parse_cli("=:").is_err());
        assert!(SecretBinding::parse_cli(":host").is_err());
    }

    #[test]
    fn test_is_host_allowed_basic() {
        let config = ProxyConfig {
            listen_addr: "127.0.0.1:0".parse().unwrap(),
            bindings: vec![],
            allowed_hosts: vec!["api.openai.com".to_string()],
            blocked_hosts: vec!["evil.com".to_string()],
            allowlist_only: false,
            sandbox_name: "test".to_string(),
            hooks: vec![],
            llm_intercept: false,
            llm_domains: vec![],
            org_managed_domains: vec![],
            llm_governance: None,
        };

        assert!(is_host_allowed("api.openai.com", &config));
        assert!(is_host_allowed("example.com", &config)); // not blocked, not allowlist_only
        assert!(!is_host_allowed("evil.com", &config));
    }

    #[test]
    fn test_is_host_allowed_allowlist_only() {
        let config = ProxyConfig {
            listen_addr: "127.0.0.1:0".parse().unwrap(),
            bindings: vec![],
            allowed_hosts: vec!["api.openai.com".to_string(), "*.anthropic.com".to_string()],
            blocked_hosts: vec![],
            allowlist_only: true,
            sandbox_name: "test".to_string(),
            hooks: vec![],
            llm_intercept: false,
            llm_domains: vec![],
            org_managed_domains: vec![],
            llm_governance: None,
        };

        assert!(is_host_allowed("api.openai.com", &config));
        assert!(is_host_allowed("api.anthropic.com", &config));
        assert!(!is_host_allowed("evil.com", &config));
        assert!(!is_host_allowed("example.com", &config));
    }

    #[test]
    fn test_is_host_allowed_wildcard() {
        let config = ProxyConfig {
            listen_addr: "127.0.0.1:0".parse().unwrap(),
            bindings: vec![],
            allowed_hosts: vec!["*.openai.com".to_string()],
            blocked_hosts: vec![],
            allowlist_only: true,
            sandbox_name: "test".to_string(),
            hooks: vec![],
            llm_intercept: false,
            llm_domains: vec![],
            org_managed_domains: vec![],
            llm_governance: None,
        };

        assert!(is_host_allowed("api.openai.com", &config));
        assert!(is_host_allowed("files.openai.com", &config));
        assert!(is_host_allowed("openai.com", &config)); // bare domain matches too
        assert!(!is_host_allowed("api.anthropic.com", &config));
    }

    #[test]
    fn test_is_host_allowed_with_port() {
        let config = ProxyConfig {
            listen_addr: "127.0.0.1:0".parse().unwrap(),
            bindings: vec![],
            allowed_hosts: vec!["api.openai.com".to_string()],
            blocked_hosts: vec![],
            allowlist_only: true,
            sandbox_name: "test".to_string(),
            hooks: vec![],
            llm_intercept: false,
            llm_domains: vec![],
            org_managed_domains: vec![],
            llm_governance: None,
        };

        assert!(is_host_allowed("api.openai.com:443", &config));
    }

    #[test]
    fn test_matches_domain() {
        assert!(matches_domain("api.openai.com", "api.openai.com"));
        assert!(matches_domain("api.openai.com", "*.openai.com"));
        assert!(matches_domain("openai.com", "*.openai.com"));
        assert!(!matches_domain("api.anthropic.com", "*.openai.com"));
        assert!(!matches_domain("evil.com", "api.openai.com"));
    }

    #[test]
    fn test_generate_proxy_ca() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (cert_pem, signer) = generate_proxy_ca().unwrap();
        assert!(cert_pem.contains("BEGIN CERTIFICATE"));
        // Verify we can sign a host cert with the CA
        let tls_config = generate_host_tls_config("api.openai.com", &signer).unwrap();
        assert!(Arc::strong_count(&tls_config) == 1);
    }
}
