# AgentKernel Desktop App — Tauri 2

**Branch**: `feat/desktop-app`
**Location**: `app/` at repo root
**Architecture**: Tauri 2 Rust backend + React 19 + TypeScript + Vite + Tailwind + shadcn/ui

## Key Design Decisions

- **Decoupled from main crate** — `app/` has its own `Cargo.toml`, NOT a workspace member. Talks to agentkernel via HTTP API only (user runs `agentkernel serve` separately).
- **Tauri commands in Rust** call the agentkernel HTTP API via `reqwest`. Frontend never makes direct HTTP calls — all data flows through Tauri IPC (`invoke`).
- **React Query** for data fetching + polling (3s interval). No heavy state management.
- **MVP scope**: sandbox lifecycle, exec, dashboard, snapshots, templates, settings. No enterprise/policy/pipeline features.

## Directory Structure

```
app/
├── src-tauri/
│   ├── Cargo.toml                 # agentkernel-desktop crate
│   ├── tauri.conf.json            # Window config, build config
│   ├── build.rs                   # tauri_build::build()
│   ├── capabilities/default.json  # Tauri 2 permissions
│   ├── icons/                     # App icons
│   └── src/
│       ├── main.rs                # Entry point
│       ├── lib.rs                 # Tauri builder, command registration
│       ├── api_client.rs          # reqwest HTTP client → agentkernel API
│       ├── types.rs               # Shared types (mirrors sdk/rust types + Serialize)
│       ├── state.rs               # AppState: Settings + ApiClient in Mutex
│       └── commands/
│           ├── mod.rs
│           ├── health.rs          # check_connection
│           ├── sandboxes.rs       # list, get, create, remove, extend_ttl
│           ├── exec.rs            # exec, detached exec/logs/kill
│           ├── snapshots.rs       # list, take, delete, restore
│           ├── templates.rs       # list built-in templates
│           └── settings.rs        # get/save settings to disk
├── src/                           # React frontend
│   ├── main.tsx                   # React entry
│   ├── App.tsx                    # Router setup
│   ├── lib/
│   │   ├── api.ts                 # Typed invoke() wrappers
│   │   ├── types.ts               # TS types matching Rust types
│   │   ├── utils.ts               # Formatting helpers
│   │   └── hooks/
│   │       ├── use-sandboxes.ts   # React Query: list sandboxes (polls)
│   │       ├── use-sandbox.ts     # React Query: single sandbox
│   │       ├── use-snapshots.ts
│   │       ├── use-templates.ts
│   │       ├── use-health.ts      # Connection polling
│   │       ├── use-settings.ts
│   │       └── use-exec.ts        # Exec mutation
│   ├── components/
│   │   ├── layout/
│   │   │   ├── sidebar.tsx        # Nav: Dashboard, Sandboxes, Templates, Snapshots, Settings
│   │   │   ├── header.tsx         # Connection status dot + theme toggle
│   │   │   └── app-shell.tsx      # Sidebar + header + content area
│   │   ├── ui/                    # shadcn/ui components (button, card, table, dialog, etc.)
│   │   ├── sandbox/
│   │   │   ├── sandbox-table.tsx
│   │   │   ├── sandbox-create-dialog.tsx
│   │   │   ├── sandbox-status-badge.tsx
│   │   │   ├── sandbox-detail-panel.tsx
│   │   │   ├── sandbox-info-card.tsx
│   │   │   ├── sandbox-exec-terminal.tsx
│   │   │   └── sandbox-actions.tsx
│   │   ├── snapshot/
│   │   │   ├── snapshot-table.tsx
│   │   │   └── snapshot-restore-dialog.tsx
│   │   ├── template/
│   │   │   ├── template-grid.tsx
│   │   │   └── template-create-dialog.tsx
│   │   ├── dashboard/
│   │   │   ├── status-cards.tsx
│   │   │   ├── recent-sandboxes.tsx
│   │   │   └── quick-actions.tsx
│   │   └── settings/
│   │       └── settings-form.tsx
│   ├── pages/
│   │   ├── dashboard.tsx
│   │   ├── sandboxes.tsx
│   │   ├── sandbox-detail.tsx
│   │   ├── templates.tsx
│   │   ├── snapshots.tsx
│   │   └── settings.tsx
│   └── styles/globals.css
├── index.html
├── package.json
├── vite.config.ts
├── tailwind.config.ts
├── postcss.config.js
├── tsconfig.json
└── components.json              # shadcn/ui config
```

## Key Dependencies

**Rust (src-tauri/Cargo.toml)**:

- `tauri = "2"`, `tauri-build = "2"`, `tauri-plugin-shell = "2"`
- `reqwest = { version = "0.12", features = ["json", "rustls-tls"] }`
- `serde`, `serde_json`, `tokio`, `anyhow`, `dirs`, `chrono`

**Frontend (package.json)**:

- `@tauri-apps/api ^2`, `@tauri-apps/plugin-shell ^2`
- `react ^19`, `react-dom ^19`, `react-router-dom ^7`
- `@tanstack/react-query ^5`
- `lucide-react`, `date-fns`, `clsx`, `tailwind-merge`, `class-variance-authority`
- Radix UI primitives (via shadcn/ui)
- Dev: `@tauri-apps/cli ^2`, `vite ^6`, `typescript ^5`, `tailwindcss ^3`

## Screens

| Route              | Page           | Key Components                                                                                              |
| ------------------ | -------------- | ----------------------------------------------------------------------------------------------------------- |
| `/`                | Dashboard      | Status cards (running/stopped/total), recent sandboxes, quick actions, connection indicator                 |
| `/sandboxes`       | Sandbox List   | Sortable table, status badges, create dialog, action dropdown (start/stop/remove/snapshot)                  |
| `/sandboxes/:name` | Sandbox Detail | Tabs: Info (metadata cards, TTL), Exec (terminal input+output, detached commands), Files (read/write), Logs |
| `/templates`       | Templates      | Grid of cards grouped by category (agents, languages, specialized), click to create                         |
| `/snapshots`       | Snapshots      | Table with restore/delete actions                                                                           |
| `/settings`        | Settings       | API URL, API key, theme, poll interval, test connection button                                              |

## Implementation Order

### Phase 1 — Scaffold + Rust Backend

1. Create branch `feat/desktop-app`
2. Scaffold `app/` — Tauri 2 project with React+Vite+TS template
3. Configure Tailwind + shadcn/ui
4. `src-tauri/src/types.rs` — mirror types from `sdk/rust/src/types.rs`, add Serialize
5. `src-tauri/src/api_client.rs` — reqwest client modeled on `sdk/rust/src/client.rs`
6. `src-tauri/src/state.rs` — AppState with Settings + ApiClient
7. `src-tauri/src/commands/*.rs` — all Tauri command handlers
8. `src-tauri/src/lib.rs` — wire commands into Tauri builder
9. Verify: `cd app && cargo build --manifest-path src-tauri/Cargo.toml`

### Phase 2 — Frontend Foundation

10. `src/lib/types.ts` + `src/lib/api.ts` — typed invoke wrappers
11. `src/lib/hooks/*.ts` — React Query hooks with polling
12. Layout components (sidebar, header, app-shell)
13. Settings page (first end-to-end flow)
14. `npm run tauri dev` — verify Tauri window launches

### Phase 3 — Core Pages

15. Dashboard page
16. Sandbox list + create dialog
17. Sandbox detail (Info + Exec tabs)
18. Snapshots page
19. Templates page

### Phase 4 — Polish

20. Dark/light theme support
21. Empty states, loading skeletons, error states
22. Toast notifications for actions
23. Update root `.gitignore` for `app/src-tauri/target/`, `app/node_modules/`, `app/dist/`
24. Verify full build: `cd app && npm run tauri build`

## Reference Files

| Purpose                    | File                     |
| -------------------------- | ------------------------ |
| API endpoints              | `src/http_api.rs`        |
| OpenAPI spec               | `api/openapi.yaml`       |
| Rust SDK types (copy from) | `sdk/rust/src/types.rs`  |
| Rust SDK client (pattern)  | `sdk/rust/src/client.rs` |
| Template list              | `src/template.rs`        |
| Config schema              | `src/config.rs`          |

## Verification

1. `cd app && npm run tauri dev` — app launches, connects to running `agentkernel serve`
2. Settings page: change API URL, save, reload — persists
3. Dashboard: shows sandbox counts from live API
4. Create sandbox from list page, verify it appears
5. Open sandbox detail, run `echo hello` in exec tab, see output
6. Take snapshot, verify in snapshots page, restore it
7. `cd app && npm run tauri build` — produces .app bundle
