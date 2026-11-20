# Open an AgentKernel sandbox in Rex

Add the sandbox as a **Remote Host** in Rex for Mac. Its sidebar section opens
terminals inside the sandbox, not on the Mac running AgentKernel.

## 1. Start a sandbox with SSH

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

## 2. Add the sandbox to Rex

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
