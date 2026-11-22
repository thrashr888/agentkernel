# Open an AgentKernel sandbox in Rex

Add the sandbox as a **Remote Host** in Rex for Mac. Its sidebar section opens
terminals inside the sandbox, not on the Mac running AgentKernel.

## Local Mac app: no published ports

Use this opt-in route on Apple Silicon with AgentKernel for Mac **v0.21.3 or
later**, Rex for Mac, and Docker Desktop on the same Mac. In AgentKernel,
select an app-managed local server, usually **Local**. It exposes a Unix
socket for your macOS user, not an SSH or TCP port. If another service
already uses the local server port, give the app-managed server its own
unused loopback port.

The local owner must supply a prepared Linux ARM64 Docker image and its pinned
Rex, gate, and relay binaries. Obtain Rex from
[Superlogical](https://www.superlogical.com/rex/docs/linux/installation).
The published AgentKernel server image and stock agent templates do not
provide this prepared guest. The app does not download or bundle its assets.

To opt in, supply an owner-only `owner-rex.json` (mode `0600`) in
`~/Library/Application Support/agentkernel-desktop/` with the approval flag,
immutable local Docker image ID, SHA-256 pins for the three binaries, and
the local Docker and socket paths. See the
[manifest fields](https://github.com/thrashr888/agentkernel/blob/v0.21.3/src/docker_rex/registry.rs#L989-L999).
Restart AgentKernel after setup. Without a valid manifest, **Create Rex
Sandbox** stays hidden; use the SSH workflow below if you don't have a
prepared setup.

1. In **Sandboxes**, select **Create Rex Sandbox**, name it, and choose
   **Copy Rex Address** once it starts.
2. In Rex, choose **Add Remote Host** and paste the exact `unix:///…`
   address. Open a terminal under the new host. Rex Remote Control is not
   needed for manual setup.
3. After restarting AgentKernel, select **Get Rex Address** on the original
   sandbox before reconnecting from the saved Rex host. AgentKernel only
   reopens the original running sandbox; it won't start a replacement.

## SSH: use a Docker image with Rex installed

For the separate SSH route, you can start from a
[built-in AgentKernel template](../commands/templates.md) and prepare its
image as described below.

### 1. Start a sandbox with SSH

This example runs Docker Engine 28 or later and Rex on the same Mac. Before
creating the sandbox, verify that its Docker network
[defaults published ports to loopback](https://docs.docker.com/engine/network/port-publishing/#setting-the-default-bind-address-for-containers).
If you can't verify that, stop. Docker can otherwise publish ports on all
interfaces.

Prepare a Linux image with OpenSSH server and Superlogical's Rex binary at
`/usr/bin/rex`. Follow [Rex's Linux installation guide](https://www.superlogical.com/rex/docs/linux/installation)
for your image. Rex can start its personal server over SSH, so no separate
Rex listener is needed.

```bash
agentkernel sandbox create rex-dev --backend docker --ssh --ttl 1h \
  --image YOUR_REX_IMAGE
docker port agentkernel-rex-dev 22/tcp
agentkernel exec rex-dev -- rex version
```

Continue only if the port is loopback-only. Use its host-side port below.
On AgentKernel v0.21.3 or later, `agentkernel ssh url rex-dev` also prints
the address for the running local Docker sandbox.

### 2. Add the sandbox to Rex

Run `agentkernel ssh connect rex-dev` and exit the session. It generates a
short-lived SSH key and certificate, but skips host-key checking. For the
Rex connection, add this alias to your Mac's SSH config:

```sshconfig
Host ak-rex-dev
    HostName 127.0.0.1
    Port YOUR_MAPPED_PORT
    User sandbox
    IdentityFile ~/.agentkernel/ssh/rex-dev/client_key
    CertificateFile ~/.agentkernel/ssh/rex-dev/client_key-cert.pub
    IdentitiesOnly yes
```

Compare the fingerprint from the sandbox with the first SSH prompt:

```bash
agentkernel exec rex-dev -- cat /etc/ssh/ssh_host_ed25519_key.pub | ssh-keygen -lf -
ssh ak-rex-dev rex version
```

In Rex, choose **Add Remote Host…** and enter `ssh://sandbox@ak-rex-dev`.
To verify the new host, write a marker from the Mac:

```bash
agentkernel exec rex-dev -- sh -c 'echo inside-agentkernel > /tmp/rex-check'
```

Open a terminal under the new Rex host. `cat /tmp/rex-check` should print
`inside-agentkernel`.

When finished, save your work, run `agentkernel sandbox remove rex-dev`, and
remove the entry in Rex. Renew the client certificate with
`agentkernel ssh connect rex-dev` if it expires before you reconnect.

For a different AgentKernel host, use an existing private route to the
**sandbox's** SSH port, not the host's SSH service. See
[Rex remote access](https://www.superlogical.com/rex/docs/linux/remote-access)
and [AgentKernel SSH settings](../config/toml.md#ssh).
