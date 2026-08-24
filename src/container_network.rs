//! Managed bridge networking for the local Docker and Podman backends.
//!
//! The allocator in this module deliberately owns only the IP leases that it
//! writes to disk.  A bridge that already exists without the AgentKernel
//! ownership label is treated as external and is never removed by us.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

#[cfg(unix)]
use std::os::fd::AsRawFd;

const DEFAULT_SUBNET: &str = "172.30.0.0/24";
const LOCK_RETRIES: usize = 100;
const LOCK_WAIT: Duration = Duration::from_millis(10);

/// Configuration for a managed Docker/Podman bridge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedNetworkConfig {
    /// Name passed to the container runtime.
    pub name: String,
    /// IPv4 subnet in CIDR notation.
    #[serde(default = "default_subnet")]
    pub subnet: String,
    /// Optional bridge gateway. Docker/Podman choose one when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway: Option<String>,
    /// Optional DNS server addresses passed to the container runtime.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dns: Vec<String>,
    /// Optional fixed sandbox address. Dynamic leases are used otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub static_ip: Option<String>,
}

fn default_subnet() -> String {
    DEFAULT_SUBNET.to_string()
}

impl ManagedNetworkConfig {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            subnet: default_subnet(),
            gateway: None,
            dns: Vec::new(),
            static_ip: None,
        }
    }

    /// Validate every value before it reaches a runtime CLI invocation.
    pub fn validate(&self) -> Result<()> {
        validate_network_name(&self.name)?;
        let subnet = Ipv4Cidr::parse(&self.subnet)
            .with_context(|| format!("invalid managed network subnet '{}'", self.subnet))?;

        let gateway = self
            .gateway
            .as_deref()
            .map(|gateway| parse_ipv4(gateway, "gateway"))
            .transpose()?
            .unwrap_or_else(|| Ipv4Addr::from(subnet.first_host()));
        validate_host_address(gateway, subnet, "gateway")?;
        for dns in &self.dns {
            parse_ipv4(dns, "DNS server")?;
        }
        if let Some(static_ip) = self.static_ip.as_deref() {
            let static_ip = parse_ipv4(static_ip, "static IP")?;
            validate_host_address(static_ip, subnet, "static IP")?;
            if gateway == static_ip {
                bail!(
                    "static IP '{}' cannot equal the managed network gateway",
                    static_ip
                );
            }
        }
        Ok(())
    }

    /// Resolve the gateway used by AgentKernel. When omitted, the first host
    /// address is always supplied to the runtime so allocation never relies
    /// on an unverified runtime default.
    pub fn effective_gateway(&self) -> Result<String> {
        self.validate()?;
        let subnet = Ipv4Cidr::parse(&self.subnet)?;
        Ok(self
            .gateway
            .clone()
            .unwrap_or_else(|| Ipv4Addr::from(subnet.first_host()).to_string()))
    }

    pub fn with_overrides(
        name: Option<String>,
        subnet: Option<String>,
        gateway: Option<String>,
        dns: Vec<String>,
        static_ip: Option<String>,
    ) -> Result<Option<Self>> {
        if name.is_none()
            && subnet.is_none()
            && gateway.is_none()
            && dns.is_empty()
            && static_ip.is_none()
        {
            return Ok(None);
        }
        let name = name.ok_or_else(|| anyhow::anyhow!("managed network name is required"))?;
        let mut config = Self::new(name);
        if let Some(subnet) = subnet {
            config.subnet = subnet;
        }
        config.gateway = gateway;
        config.dns = dns;
        config.static_ip = static_ip;
        config.validate()?;
        Ok(Some(config))
    }
}

/// Validate the runtime's network-name grammar without invoking Docker.
pub fn validate_network_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 63 {
        bail!("managed network name must contain 1-63 characters");
    }
    if !name
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        bail!(
            "managed network name '{}' contains unsupported characters",
            name
        );
    }
    if name.starts_with('.') || name.starts_with('-') || name.starts_with('_') {
        bail!(
            "managed network name '{}' must start with a letter or digit",
            name
        );
    }
    Ok(())
}

fn parse_ipv4(value: &str, field: &str) -> Result<Ipv4Addr> {
    let address = value
        .parse::<IpAddr>()
        .with_context(|| format!("invalid {field} '{value}'; expected an IPv4 address"))?;
    match address {
        IpAddr::V4(address) => Ok(address),
        IpAddr::V6(_) => bail!("{field} '{value}' must be an IPv4 address"),
    }
}

fn validate_host_address(address: Ipv4Addr, subnet: Ipv4Cidr, field: &str) -> Result<()> {
    let value = u32::from(address);
    if !subnet.contains(value) {
        bail!("{field} '{}' is outside subnet {:?}", address, subnet);
    }
    if value == subnet.network() || value == subnet.broadcast() {
        bail!(
            "{field} '{}' cannot be the subnet or broadcast address",
            address
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Ipv4Cidr {
    network: u32,
    prefix: u8,
}

impl Ipv4Cidr {
    fn parse(value: &str) -> Result<Self> {
        let (address, prefix) = value
            .split_once('/')
            .ok_or_else(|| anyhow::anyhow!("CIDR must include a prefix length"))?;
        let address = address
            .parse::<Ipv4Addr>()
            .with_context(|| format!("invalid IPv4 subnet address '{address}'"))?;
        let prefix = prefix
            .parse::<u8>()
            .with_context(|| format!("invalid CIDR prefix '{prefix}'"))?;
        if prefix > 32 {
            bail!("CIDR prefix must be between 0 and 32");
        }
        let mask = if prefix == 0 {
            0
        } else {
            u32::MAX << (32 - prefix)
        };
        let network = u32::from(address) & mask;
        if network != u32::from(address) {
            bail!("subnet '{}' must use its network address", value);
        }
        Ok(Self { network, prefix })
    }

    fn mask(self) -> u32 {
        if self.prefix == 0 {
            0
        } else {
            u32::MAX << (32 - self.prefix)
        }
    }

    fn network(self) -> u32 {
        self.network
    }

    fn broadcast(self) -> u32 {
        self.network | !self.mask()
    }

    fn contains(self, address: u32) -> bool {
        address & self.mask() == self.network
    }

    fn first_host(self) -> u32 {
        self.network.saturating_add(1)
    }

    fn last_host(self) -> u32 {
        self.broadcast().saturating_sub(1)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct AllocationFile {
    #[serde(default)]
    networks: BTreeMap<String, NetworkAllocation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NetworkAllocation {
    config: ManagedNetworkConfig,
    #[serde(default)]
    leases: BTreeMap<String, String>,
}

/// An address reserved for one sandbox on a managed bridge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedNetworkLease {
    pub network: ManagedNetworkConfig,
    pub ip: String,
}

/// Durable, process-safe address allocator.
#[derive(Debug, Clone)]
pub struct NetworkAllocator {
    data_dir: PathBuf,
}

impl NetworkAllocator {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
        }
    }

    pub fn reserve(
        &self,
        sandbox: &str,
        config: &ManagedNetworkConfig,
    ) -> Result<ManagedNetworkLease> {
        config.validate()?;
        let _lock = AllocationLock::acquire(&self.lock_path())?;
        fs::create_dir_all(&self.data_dir)?;
        let mut file = self.load()?;
        let subnet = Ipv4Cidr::parse(&config.subnet)?;
        // Static IP is a per-sandbox lease, not a property of the shared
        // bridge. Keep it out of the persisted network identity so multiple
        // sandboxes can use one bridge with different fixed addresses.
        let mut network_identity = config.clone();
        network_identity.static_ip = None;
        let gateway = config.effective_gateway()?;
        let network =
            file.networks
                .entry(config.name.clone())
                .or_insert_with(|| NetworkAllocation {
                    config: network_identity.clone(),
                    leases: BTreeMap::new(),
                });
        if network.config != network_identity {
            bail!(
                "managed network '{}' configuration conflicts with a persisted allocation",
                config.name
            );
        }
        let ip = if let Some(existing) = network.leases.get(sandbox) {
            if let Some(requested) = config.static_ip.as_deref()
                && existing != requested
            {
                bail!(
                    "sandbox '{}' already has managed network address '{}'; refusing to change it to static IP '{}'",
                    sandbox,
                    existing,
                    requested
                );
            }
            existing.clone()
        } else {
            let ip = if let Some(static_ip) = &config.static_ip {
                let address = parse_ipv4(static_ip, "static IP")?;
                if network.leases.values().any(|lease| lease == static_ip) {
                    bail!("static IP '{}' is already allocated", static_ip);
                }
                address.to_string()
            } else {
                let mut selected = None;
                for value in subnet.first_host()..=subnet.last_host() {
                    let candidate = Ipv4Addr::from(value).to_string();
                    if gateway == candidate
                        || network.leases.values().any(|lease| lease == &candidate)
                    {
                        continue;
                    }
                    selected = Some(candidate);
                    break;
                }
                selected.ok_or_else(|| {
                    anyhow::anyhow!("managed network '{}' has no free addresses", config.name)
                })?
            };
            network.leases.insert(sandbox.to_string(), ip.clone());
            ip
        };
        self.save(&file)?;
        Ok(ManagedNetworkLease {
            network: config.clone(),
            ip,
        })
    }

    pub fn release(&self, sandbox: &str, config: &ManagedNetworkConfig) -> Result<()> {
        let _lock = AllocationLock::acquire(&self.lock_path())?;
        let mut file = self.load()?;
        let mut remove_network = false;
        if let Some(network) = file.networks.get_mut(&config.name) {
            network.leases.remove(sandbox);
            remove_network = network.leases.is_empty();
        }
        if remove_network {
            file.networks.remove(&config.name);
        }
        self.save(&file)
    }

    fn allocation_path(&self) -> PathBuf {
        self.data_dir.join("container-network-allocations.json")
    }

    fn lock_path(&self) -> PathBuf {
        self.data_dir.join("container-network-allocations.lock")
    }

    fn load(&self) -> Result<AllocationFile> {
        let path = self.allocation_path();
        if !path.exists() {
            return Ok(AllocationFile::default());
        }
        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("failed to parse {}", path.display()))
    }

    fn save(&self, file: &AllocationFile) -> Result<()> {
        fs::create_dir_all(&self.data_dir)?;
        let path = self.allocation_path();
        let temp = path.with_extension(format!("json.{}", std::process::id()));
        let mut temp_file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temp)?;
        temp_file.write_all(&serde_json::to_vec_pretty(file)?)?;
        temp_file.sync_all()?;
        fs::rename(&temp, &path)?;
        #[cfg(unix)]
        File::open(&self.data_dir)?.sync_all()?;
        Ok(())
    }
}

struct AllocationLock {
    #[cfg(not(unix))]
    path: PathBuf,
    _file: File,
}

impl AllocationLock {
    fn acquire(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        #[cfg(unix)]
        {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(path)?;
            for _ in 0..LOCK_RETRIES {
                // The lock is advisory and tied to this open file description;
                // no process can unlink another process's active lock path.
                let result =
                    unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
                if result == 0 {
                    return Ok(Self { _file: file });
                }
                let error = std::io::Error::last_os_error();
                if !error
                    .raw_os_error()
                    .is_some_and(|code| code == libc::EAGAIN || code == libc::EWOULDBLOCK)
                {
                    return Err(error.into());
                }
                thread::sleep(LOCK_WAIT);
            }
            bail!("timed out waiting for managed network allocation lock")
        }

        #[cfg(not(unix))]
        {
            for _ in 0..LOCK_RETRIES {
                match OpenOptions::new().write(true).create_new(true).open(path) {
                    Ok(file) => {
                        return Ok(Self {
                            path: path.to_path_buf(),
                            _file: file,
                        });
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                        thread::sleep(LOCK_WAIT);
                    }
                    Err(error) => return Err(error.into()),
                }
            }
            bail!("timed out waiting for managed network allocation lock")
        }
    }
}

#[cfg(not(unix))]
impl Drop for AllocationLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn validates_bridge_fields_and_rejects_unsafe_values() {
        let mut config = ManagedNetworkConfig::new("agent-dev");
        config.subnet = "10.10.0.0/24".to_string();
        config.gateway = Some("10.10.0.1".to_string());
        config.dns = vec!["1.1.1.1".to_string()];
        config.static_ip = Some("10.10.0.9".to_string());
        config.validate().unwrap();

        config.static_ip = Some("10.11.0.9".to_string());
        assert!(config.validate().is_err());
        config.static_ip = Some("10.10.0.255".to_string());
        assert!(config.validate().is_err());
        config.name = "bad name".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn allocator_is_durable_and_collision_safe() {
        let temp = TempDir::new().unwrap();
        let allocator = NetworkAllocator::new(temp.path());
        let config = ManagedNetworkConfig::new("agent-dev");
        let first = allocator.reserve("one", &config).unwrap();
        let restarted = NetworkAllocator::new(temp.path());
        let same = restarted.reserve("one", &config).unwrap();
        assert_eq!(first, same);
        let second = restarted.reserve("two", &config).unwrap();
        assert_ne!(first.ip, second.ip);
        restarted.release("one", &config).unwrap();
        assert_eq!(restarted.reserve("three", &config).unwrap().ip, first.ip);
    }

    #[test]
    fn static_addresses_are_exclusive() {
        let temp = TempDir::new().unwrap();
        let allocator = NetworkAllocator::new(temp.path());
        let mut config = ManagedNetworkConfig::new("agent-dev");
        config.static_ip = Some("172.30.0.9".to_string());
        allocator.reserve("one", &config).unwrap();
        assert!(allocator.reserve("two", &config).is_err());
        config.static_ip = Some("172.30.0.10".to_string());
        assert_eq!(allocator.reserve("two", &config).unwrap().ip, "172.30.0.10");
    }

    #[test]
    fn existing_static_lease_cannot_change_address() {
        let temp = TempDir::new().unwrap();
        let allocator = NetworkAllocator::new(temp.path());
        let mut config = ManagedNetworkConfig::new("agent-dev");
        config.static_ip = Some("172.30.0.9".to_string());
        allocator.reserve("one", &config).unwrap();
        config.static_ip = Some("172.30.0.10".to_string());
        let error = allocator.reserve("one", &config).unwrap_err().to_string();
        assert!(error.contains("refusing to change"));
    }

    #[test]
    fn allocation_lock_is_process_safe() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("allocations.lock");
        let first = AllocationLock::acquire(&path).unwrap();
        assert!(AllocationLock::acquire(&path).is_err());
        drop(first);
        assert!(AllocationLock::acquire(&path).is_ok());
    }
}
