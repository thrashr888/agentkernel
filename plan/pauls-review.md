# Paul's Review

## Already Fixed

- ~~created and IP don't show on the sandbox list~~ — fixed in 6faacf6 (requires server restart to pick up new binary)
- ~~dark mode isn't dark~~ — AppShell now reads settings.theme and toggles `dark` class on root element

## Bugs

- restore snapshot doesn't work?
  - Backend handler exists (`POST /snapshots/:name/restore`). Frontend calls `api.restoreSnapshot(name)`.
    The handler reads the snapshot metadata then calls `manager.create()` with a new name. Could be
    failing silently — need to add toast feedback to see the error. Might also be a Docker image_tag
    mismatch if the snapshot was taken from a different backend version.
  - **Confirmed**: restored sandbox `my-ssh-box-restored` has `image: "agentkernel-snap:test-snap"`
    which is a local Docker tag. The apple backend tries to pull it from Docker Hub → 401 Unauthorized.
    Restore needs to handle cross-backend image references or re-tag for the target backend.
  - **Action**: add toast on success/error, then fix snapshot restore for apple backend.

- no error handling / feedback on actions
  - Start, stop, remove, snapshot — all fail silently if the API returns an error. The user sees
    nothing happen and no indication of why. Example: starting `my-ssh-box-restored` returns
    `401 Unauthorized` from Docker Hub but the UI just stays on "stopped".
  - **Action**: add toast notifications on every mutation success/error across all pages. See
    "need some kind of notice when actions are taken" in Quick Wins below — these are the same issue.

- sandbox logs tab — can it show what's already been running in the sandbox?
  - Currently the Logs tab only shows _detached_ background commands (ones started via `exec-detach`).
    If you ran commands via the Exec tab or CLI `exec`, those are fire-and-forget and don't appear.
  - The backend has `GET /sandboxes/:name/logs` (`handle_sandbox_logs`) which returns Docker container
    logs. This is the actual stdout/stderr history of the sandbox process itself.
  - **Action**: add a "Container Logs" section to the Logs tab that fetches `/sandboxes/:name/logs`
    separately from the detached jobs list. This would show what's been happening in the sandbox.

## Quick Wins

- the app window should remember its size between app restarts.

- create sandbox - help message should say what the policy options do
  - The security profile dropdown just shows "Permissive / Moderate / Restrictive" with no explanation.
  - Add `<SelectItem>` descriptions or a small help text below the dropdown with the matrix:
    permissive = network + mount cwd + mount home + pass env;
    moderate = network only;
    restrictive = no network, no mounts, read-only fs.

- don't need a save button: just save it on blur
  - Settings page has an explicit Save button. Switch to auto-save: debounce on each field change
    (or save on blur). Remove the Save button entirely. Show a subtle "Saved" indicator.
  - Note: Test Connection can stay as a button — that's an explicit action, not a setting change.

- need some kind of notice when actions are taken
  - We have `<Toaster />` mounted in AppShell but zero mutations show toasts.
  - **Action**: add `onSuccess`/`onError` toast calls to every mutation across all pages:
    create/remove/start/stop sandbox, take/restore/delete snapshot, extend TTL, save settings.
  - Pattern: `toast({ title: "Sandbox removed", description: name })` on success,
    `toast({ title: "Failed to remove sandbox", description: error.message, variant: "destructive" })` on error.

## Medium Features

- add more sandbox commands/actions to the sandbox detail
  - start/stop — start button when stopped, stop button when running. We have `start_sandbox` and
    `stop_sandbox` Tauri commands wired up. Just need UI buttons on the detail page header.
  - export — could mean `docker export` (tarball of filesystem). Backend has `cp` but no export
    endpoint yet. Would need a new `POST /sandboxes/:name/export` handler. Lower priority.
  - ssh connection string — backend has `ssh` and `ssh-config` CLI commands. Could show the
    connection string: `ssh -i ~/.agentkernel/keys/id_ed25519 sandbox@<ip>`.
    Need a new Tauri command that returns the SSH config for a given sandbox.
  - open in terminal — Tauri 2 has `tauri-plugin-shell` which can `open()` terminal apps.
    On macOS: `open -a Terminal` or spawn `osascript` to open iTerm. Would run `agentkernel attach <name>`.

- should be able to pick a backend when using a template? what other options?
  - Templates currently hard-code image/vcpus/memory_mb and create with defaults.
  - Options to expose in the template create dialog: backend (docker/firecracker — once firecracker
    ships), security profile, network toggle, ports, custom resource overrides.
  - Start with just security profile since that's the most useful one today.

- long running operations should show up in a queue
  - Operations like create sandbox, take snapshot, restore snapshot can take seconds.
  - Could use a persistent notification/activity panel (bottom drawer or sidebar section) that shows
    in-flight mutations with spinners. React Query's `useIsMutating()` can track global mutation count.
  - Simpler first pass: just use toasts with "Creating sandbox..." → "Sandbox created" pattern.

## Bigger Features

- macOS menubar app to show: connection status, latest agents, what else?
  - Tauri 2 supports system tray via `tauri-plugin-tray`. Can show:
    connection status (green/red dot), running sandbox count, quick create, recent sandboxes.
  - This is a separate implementation effort on top of the existing app.

- settings page can show stats/doctor/status output
  - CLI has `agentkernel status`, `agentkernel doctor`, `agentkernel stats`.
  - None of these have HTTP API endpoints yet. Would need:
    `GET /status` — installation status (firecracker, kernel, rootfs paths)
    `GET /doctor` — health checks (KVM available, Docker running, etc.)
    `GET /stats` — audit log aggregations (total sandboxes created, exec count, uptime)
  - Could add a "Diagnostics" card to Settings page, or a separate page.
  - Add the version to the bottom

- show plugin/agent list and allow installing/uninstalling plugins from the UI
  - CLI has `agentkernel plugin list`, `agentkernel plugin install <agent>`, `agentkernel agents`.
  - Would need HTTP endpoints: `GET /plugins`, `POST /plugins/:name/install`,
    `DELETE /plugins/:name`, `GET /agents`.
  - A "Plugins" page with toggle switches for each agent integration.

- session management: record and replay
  - CLI has `agentkernel replay` (asciicast v2). Recording is done by the agent runners.
  - Would need: `GET /sessions`, `GET /sessions/:id` (metadata), `GET /sessions/:id/cast` (asciicast data).
  - A built-in asciicast player in the app (or use xterm.js with an asciicast addon).

- turn server/daemon on/off from the UI
  - The app currently expects `agentkernel serve` to be running externally.
  - Could use Tauri's sidecar feature to bundle the `agentkernel` binary and manage it as a child
    process. Start on app launch, stop on quit. Show toggle in settings or menubar.
  - Alternatively, use `tauri-plugin-shell` to spawn/kill the process.

- policy management
  - Enterprise feature behind `--features enterprise`. CLI has `agentkernel policy`.
  - HTTP API has `GET /policy/status` and `POST /policy/check`.
  - Would need a Policy page showing current rules, allow editing TOML-based policies.
  - Gate the UI behind the enterprise feature — hide the nav item if the endpoint returns 404.

- sparkle for updates
  - Tauri 2 has `tauri-plugin-updater` with built-in update checking and install.
  - Needs a release server (GitHub Releases works). Configure in `tauri.conf.json`.
  - Low effort to add once we have a release pipeline.

## Styling / Branding

- black & white American Apparel Helvetica kind of vibe
  - Strip color from non-status elements. Primary/accent should be black/white only.
  - Switch font stack to Helvetica Neue / Helvetica / Arial (system sans-serif).
  - Status colors stay: green for running, red for stopped/error, yellow for pending.
  - Remove blue/purple accent colors from buttons, links, badges, tab highlights.
  - Cards, borders, separators: use neutral grays only.
  - Dark mode: pure black (#000) background, white text, gray borders. Not the default slate.
  - **Action**: update tailwind.config.ts theme, globals.css, and component color classes.

## CLI vs Desktop App Coverage Assessment

Every `agentkernel` CLI command mapped against what the desktop app currently supports.

### Fully Covered (in app)

| CLI Command | App Coverage |
|-------------|-------------|
| `list` | Sandboxes page — table with name, status, image, IP, created |
| `create` | Sandboxes page — create dialog with name, image, vcpus, memory, profile |
| `start` | Sandboxes page — dropdown action (just wired up) |
| `stop` | Sandboxes page — dropdown action (just wired up) |
| `remove` | Sandboxes page — dropdown action; also on detail page |
| `info` | Sandbox detail page — Info tab with all metadata cards |
| `extend-ttl` | Sandbox detail page — Extend TTL dialog |
| `exec` | Sandbox detail page — Exec tab with command input + output |
| `exec-list` | Sandbox detail page — Logs tab shows detached jobs |
| `exec-logs` | Sandbox detail page — Logs tab shows selected job stdout/stderr |
| `exec-kill` | Sandbox detail page — Kill button on running jobs |
| `template` | Templates page — grid grouped by category, click-to-create |
| `serve` | N/A — the app is a client *of* serve; this is the prerequisite |

### Partially Covered

| CLI Command | What's There | What's Missing |
|-------------|-------------|----------------|
| `snapshot` (take) | Sandboxes page dropdown | No dedicated snapshot-from-detail button |
| `snapshot` (list/delete/restore) | Snapshots page | Restore is broken on apple backend (filed as bug) |
| `exec` (detached) | Can view detached jobs + logs | Cannot *start* detached commands from UI |
| Container logs | Logs tab exists | Only shows detached jobs, not `GET /sandboxes/:name/logs` container output (filed as bug) |

### Not Covered — Should Be in MVP

| CLI Command | Why It Matters | Effort |
|-------------|---------------|--------|
| `attach` / `ssh` | Core workflow — interactive shell access to sandbox. Users expect "Open Terminal" on a running sandbox. | Medium — needs `tauri-plugin-shell` to spawn terminal |
| `ssh-config` | IDE integration string — "Copy SSH config" would be very useful on detail page | Small — just format + copy-to-clipboard |
| `cp` | File transfer in/out of sandbox. API has file read/write/delete endpoints already. | Medium — needs a file browser component |
| `doctor` | "Why isn't this working?" is the first thing users ask. Should be on Settings page. | Small — needs one new HTTP endpoint |
| `status` | Installation status — what backends are available, versions. | Small — needs one new HTTP endpoint |

### Not Covered — Future / Nice to Have

| CLI Command | Notes |
|-------------|-------|
| `setup` | One-time install wizard. Could be a first-run flow in the app. |
| `init` | Creates `agentkernel.toml` in CWD. CLI-specific, not useful in desktop app. |
| `run` | Ephemeral sandbox (create+exec+destroy). Could be a "Quick Run" feature. |
| `mcp-server` | Stdio-based for Claude Code. Not relevant to desktop app. |
| `ssh-proxy` | ProxyCommand for SSH config. CLI plumbing, not user-facing. |
| `agents` | List agent availability. Useful in a Plugins page. |
| `plugin` | Install/manage agent plugins. Useful in a Plugins page. |
| `daemon` | VM pool management. Could be settings toggle. |
| `audit` | Audit log viewer. Could be a dedicated page. |
| `replay` | Session replay (asciicast). Needs a player component. |
| `stats` | Usage statistics from audit log. Could be on settings/diagnostics. |
| `secret` | API key management. Could be on settings page. |
| `gc` | Garbage collect expired sandboxes. Could be a button on settings. |
| `benchmark` | Hardware benchmark. Could be on diagnostics page. |
| `parallel` | Fan-out jobs. Advanced feature, lower priority. |
| `export` | Export sandbox filesystem as tar. Could be detail page action. |
| `export-config` / `import-config` | TOML config sharing. Could be on detail page. |
| `images` | Docker image cache management. Could be a settings subsection. |
| `policy` | Enterprise policy. Gated behind feature flag. |

### HTTP API Endpoints Not Used by App

| Endpoint | What It Does | Should App Use It? |
|----------|-------------|-------------------|
| `POST /run` | One-shot run (create+exec+destroy) | Yes — "Quick Run" feature |
| `POST /run/stream` | Streaming run output | Yes — better exec experience |
| `POST /batch/run` | Batch parallel execution | Maybe — advanced feature |
| `GET /sandboxes/:name/logs` | Container stdout/stderr history | **Yes** — filed as bug, should show in Logs tab |
| `POST /sandboxes/:name/files` | Batch file write | Yes — for file browser |
| `GET /sandboxes/:name/files/*` | Read file from sandbox | Yes — for file browser |
| `PUT /sandboxes/:name/files/*` | Write file to sandbox | Yes — for file browser |
| `DELETE /sandboxes/:name/files/*` | Delete file from sandbox | Yes — for file browser |
| `GET /policy/status` | Policy status | Later — enterprise only |
| `POST /policy/check` | Policy check | Later — enterprise only |

### Summary

The app covers **~60% of what a basic user would expect**. The core sandbox CRUD + exec loop works. The biggest gaps for an MVP are:

1. **No error feedback** — actions fail silently (P1 bug, filed)
2. **No terminal/SSH access** — the #1 thing power users will want
3. **No file browser** — API endpoints exist but no UI
4. **No container logs** — only detached jobs visible (P2 bug, filed)
5. **No diagnostics** — "doctor" and "status" for troubleshooting

The "bigger features" (plugins, sessions, policy, daemon control) are all reasonable V2 items.
