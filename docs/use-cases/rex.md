# Open an AgentKernel sandbox in Rex

Add the sandbox as a **Remote Host** in Rex. It gets its own sidebar section,
and terminals under it run as the sandbox user, not on the machine running
AgentKernel. You don't need a Rex or MCP plugin for this.

## 1. Prepare a Linux sandbox

Use a dedicated sandbox on an existing private AgentKernel host. This example
targets the Docker backend and assumes its published ports are already
restricted to loopback or a private network controlled by the operator. Docker
can publish mapped ports on all interfaces by default. Do not run this example
on an unrestricted host, and do not expose the sandbox SSH or Rex port to the
internet. See [Docker's port publishing documentation](https://docs.docker.com/engine/network/port-publishing/).

Prepare a Linux image with Superlogical's Rex CLI installed at a location
available to non-interactive SSH sessions, such as `/usr/bin/rex`. Use the
[Rex Linux installation guide](https://www.superlogical.com/rex/docs/linux/installation)
for your distribution and architecture. Include OpenSSH server and a writable
home for the sandbox's SSH user. Pin the image by digest for repeated use.
Rex starts the personal server over SSH when needed, so there is no need to
enable a shared Rex service or publish a separate Rex listener.

Replace the image reference below with your prepared image:

```bash
agentkernel sandbox create rex-dev --backend docker --ssh --ttl 1h \
  --image YOUR_REX_IMAGE
agentkernel sandbox info rex-dev
agentkernel exec rex-dev -- rex version
```

Use the SSH port reported for this sandbox; don't assume it is port 22 on the
host. Verify the host-side mapping before continuing (for Docker,
`docker port agentkernel-rex-dev 22/tcp`). A wildcard bind is not private
unless an existing network policy limits who can reach it. If the sandbox is
on a different machine, use an existing authenticated private route or SSH
forward to its mapped port. The destination must be **sandbox SSH**, not the
host's own SSH service.

## 2. Test and add the connection

On the AgentKernel host, `agentkernel ssh connect rex-dev` verifies the
certificate-based sandbox login and generates a short-lived client key and
certificate. In the same-machine setup, the files are already on your Rex Mac
at `~/.agentkernel/ssh/rex-dev/client_key` and `client_key-cert.pub`. If the
sandbox is on another machine, use an approved way to provision a client
credential for your Mac; never copy the sandbox's CA private key.

Rex uses the system SSH client when available. For a same-machine example,
add a host alias to your SSH config and substitute the mapped port:

```sshconfig
Host ak-rex-dev
    HostName 127.0.0.1
    Port YOUR_MAPPED_PORT
    User sandbox
    IdentityFile ~/.agentkernel/ssh/rex-dev/client_key
    CertificateFile ~/.agentkernel/ssh/rex-dev/client_key-cert.pub
    IdentitiesOnly yes
```

Keep normal SSH host-key checking enabled. Verify the fingerprint from an
independent path before trusting it. For example, read the public host key
inside the identified sandbox with
`agentkernel exec rex-dev -- cat /etc/ssh/ssh_host_ed25519_key.pub`
and compare its Ed25519 fingerprint with the first connection prompt.

```bash
ssh ak-rex-dev rex version
rex -S ssh://sandbox@ak-rex-dev ls
```

In Rex for Mac, choose **Add Remote Host…** and enter
`ssh://sandbox@ak-rex-dev`. Open a terminal under that host. To prove it is
the sandbox, create a marker through AgentKernel and read it in that Rex tab:

```bash
# In the AgentKernel host terminal:
agentkernel exec rex-dev -- sh -c 'printf "%s\n" inside-agentkernel > /tmp/rex-check'
# In the Rex terminal under the new host:
cat /tmp/rex-check
```

For reconnection after the SSH certificate expires, renew the client
credential and reconnect. A running personal server and its sessions may not
survive logout or a sandbox restart. Save work outside disposable sandboxes
before stopping them. Remove the sandbox when finished with
`agentkernel sandbox remove rex-dev`, close any temporary forwarding route,
and remove the sidebar entry in Rex. The sandbox TTL is a fallback for
environments running AgentKernel's expiry cleanup, not a substitute for
explicit teardown.

See [Rex remote access](https://www.superlogical.com/rex/docs/linux/remote-access),
[AgentKernel SSH settings](../config/toml.md#ssh), and
[AgentKernel's separate MCP server](../api/mcp.md).
