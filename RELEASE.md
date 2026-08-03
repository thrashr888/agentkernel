# Release Process

## Roll-Forward Policy

**Never delete, move, or force-push a tag.** Once a version is tagged and pushed, it is immutable. Published artifacts (crates.io, npm, PyPI, Docker images, Helm charts, GitHub Releases) cannot be reliably retracted, and consumers may already be pinned to that version.

If a release has problems, fix the issue on `main` and cut a new patch version:
- `v0.6.0` has a bug → fix and release `v0.6.1`
- Never `git tag -d v0.6.0 && git tag v0.6.0 && git push --force`

## Overview

Pushing a `v*` tag triggers four workflows in parallel:

| Workflow | File | What it does |
|----------|------|--------------|
| **Release** | `.github/workflows/release.yml` | Test gate → build CLI binaries for 4 platforms → build, sign, notarize, and staple macOS DMGs → create GitHub Release → publish Helm chart and Homebrew artifacts |
| **SDK Publish** | `.github/workflows/sdk-publish.yml` | Test gate → publish all SDKs to their registries |
| **Docker** | `.github/workflows/docker.yml` | Test gate → build and push Docker image to GHCR |

Each workflow includes its own test gate (fmt, clippy, test) that must pass before any build or publish step runs.

## Pre-Release Checklist

Before tagging, verify CI is green and all code is tested:

```bash
# 1. Main codebase quality gates
cargo fmt -- --check && cargo clippy -- -D warnings && cargo test

# 2. Rust SDK tests
cd sdk/rust && cargo test && cd ../..

# 3. Node.js SDK tests
cd sdk/nodejs && npm ci && npm run build && npm test && cd ../..

# 4. Swift SDK build
cd sdk/swift && swift build && swift test && cd ../..

# 5. Confirm CI history is green
gh run list --repo thrashr888/agentkernel --limit 5
```

All checks must pass before tagging. Fix any failures and push before proceeding. Do not tag until CI is green — tags are immutable once pushed.

## Cutting a Release

```bash
# 1. Update version in Cargo.toml (CLI)
#    SDK versions are set automatically from the tag.

# 2. Run the pre-release checklist above

# 3. Commit
git add Cargo.toml
git commit -m "release: v0.3.0"

# 4. Tag and push
git tag v0.3.0
git push origin main v0.3.0
```

Both workflows trigger on the tag push.

## What Gets Published

### CLI Binaries (release.yml)

Cross-compiled on native runners, uploaded to GitHub Releases as `.tar.gz`:

| Platform | Runner | Artifact |
|----------|--------|----------|
| Linux x64 | `ubuntu-latest` | `agentkernel-linux-x64.tar.gz` |
| Linux arm64 | `ubuntu-24.04-arm` | `agentkernel-linux-arm64.tar.gz` |
| macOS arm64 | `macos-latest` | `agentkernel-darwin-arm64.tar.gz` |
| macOS x64 | `macos-13` | `agentkernel-darwin-x64.tar.gz` |

### Desktop App (release.yml)

The two macOS desktop jobs build the Tauri app for Apple Silicon and Intel. Each
job must complete all of these steps before its artifacts are uploaded:

1. Import the Developer ID Application certificate into an ephemeral CI keychain.
2. Build with `APPLE_SIGNING_IDENTITY`.
3. Verify the app has a valid Developer ID signature and Team ID.
4. Submit the DMG to Apple’s notary service, staple the ticket, and validate it
   with `stapler` and `spctl`.

If any Apple signing or notarization secret is missing, the job fails. An
unsigned DMG cannot reach the GitHub Release or Homebrew cask.

### SDKs (sdk-publish.yml)

All 4 jobs run in parallel:

| SDK | Registry | Package Name | Auth |
|-----|----------|-------------|------|
| Node.js | npmjs.com | `agentkernel` | OIDC trusted publisher (no secret) |
| Node.js | GitHub Packages | `@thrashr888/agentkernel` | `GITHUB_TOKEN` (automatic) |
| Python | PyPI | `agentkernel-sdk` | OIDC trusted publisher (no secret) |
| Rust | crates.io | `agentkernel-sdk` | OIDC trusted publisher (no secret) |
| Swift | Git tags (no registry) | `AgentKernel` | None (verified only) |

Version is extracted from the tag name (`v0.3.0` → `0.3.0`) and injected into each SDK's manifest before publishing.

### Homebrew (manual)

The release workflow updates the formula and cask automatically after the
GitHub Release is created. For a manual correction or recovery:

```bash
# 1. Download release assets and compute SHA256 hashes
for asset in agentkernel-{darwin-arm64,darwin-x64,linux-arm64,linux-x64}.tar.gz; do
  curl -sLO "https://github.com/thrashr888/agentkernel/releases/download/v0.3.0/$asset"
  shasum -a 256 "$asset"
done

# 2. Update the formula in homebrew-agentkernel repo
#    Update version and SHA256 hashes in Formula/agentkernel.rb
#    Repo: https://github.com/thrashr888/homebrew-agentkernel

# 3. Users install/upgrade via:
brew tap thrashr888/tap
brew install agentkernel
# or: brew upgrade agentkernel
```

## One-Time Setup (per registry)

These secrets and configurations must be set up once before the first publish.

### macOS desktop signing and notarization

Add these repository secrets under **Settings → Secrets and variables →
Actions**:

| Secret | What it is |
| ------ | ---------- |
| `APPLE_CERTIFICATE` | Base64-encoded Developer ID Application `.p12` export |
| `APPLE_CERTIFICATE_PASSWORD` | Password used when exporting the `.p12` |
| `APPLE_SIGNING_IDENTITY` | Full identity, for example `Developer ID Application: Name (TEAMID)` |
| `APPLE_ID` | Apple ID email used for notarization |
| `APPLE_PASSWORD` | App-specific password for that Apple ID |
| `APPLE_TEAM_ID` | Apple Developer Team ID |
| `TAURI_SIGNING_PRIVATE_KEY` | Tauri updater private key |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Password for the Tauri updater key |

Export the Developer ID certificate without committing the resulting file:

```bash
security export -k login.keychain-db -t identities -f pkcs12 \
  -P 'ExportPassword' -o signing.p12
base64 -i signing.p12 | gh secret set APPLE_CERTIFICATE
rm signing.p12
```

Confirm the local identity before attempting a release:

```bash
security find-identity -v -p codesigning | grep 'Developer ID Application'
```

For local notarization, store an equivalent notary profile once and use it to
verify a locally built DMG:

```bash
xcrun notarytool store-credentials agentkernel-notary \
  --apple-id you@example.com --team-id YOURTEAMID
xcrun notarytool submit path/to/AgentKernel.dmg \
  --keychain-profile agentkernel-notary --wait
xcrun stapler staple path/to/AgentKernel.dmg
xcrun stapler validate path/to/AgentKernel.dmg
```

### npm (OIDC trusted publisher)

1. Go to https://www.npmjs.com/package/agentkernel/access → Trusted Publishers
2. Add a new trusted publisher:
   - Provider: **GitHub Actions**
   - Organization/username: `thrashr888`
   - Repository: `agentkernel`
   - Workflow: `sdk-publish.yml`
   - Environment: `npm`
3. Create a deployment environment in GitHub: Settings → Environments → `npm`
4. (Optional) Delete the old `NPM_TOKEN` secret once trusted publishing is verified

### PyPI (OIDC trusted publisher)

1. Publish the first version manually: `cd sdk/python && pip install build && python -m build && twine upload dist/*`
2. Go to https://pypi.org/manage/project/agentkernel-sdk/settings/publishing/ → Add a new publisher:
   - Owner: `thrashr888`
   - Repository: `agentkernel`
   - Workflow: `sdk-publish.yml`
   - Environment: `pypi`
3. Create a deployment environment in GitHub: Settings → Environments → `pypi`

### crates.io (OIDC trusted publisher)

1. Go to https://crates.io/crates/agentkernel-sdk/settings → Trusted Publishers
2. Add a new trusted publisher:
   - GitHub username: `thrashr888`
   - Repository: `agentkernel`
   - Workflow: `sdk-publish.yml`
   - Environment: `crates`
3. Create a deployment environment in GitHub: Settings → Environments → `crates`
4. (Optional) Delete the old `CARGO_REGISTRY_TOKEN` secret once trusted publishing is verified

### Swift (no setup needed)

SPM resolves packages via Git tags. Users add the dependency:

```swift
.package(url: "https://github.com/thrashr888/agentkernel.git", from: "0.3.0")
```

### GitHub Packages (no setup needed)

Uses the automatic `GITHUB_TOKEN` — no additional secrets required.

## SDK Locations

```
sdk/
  nodejs/     → npm + GitHub Packages
  python/     → PyPI
  rust/       → crates.io
  swift/      → Git tags (SPM)
```

## Verifying a Release

After tagging, check:

1. **GitHub Actions**: Both workflows should show green at https://github.com/thrashr888/agentkernel/actions
2. **GitHub Release**: Binary assets at https://github.com/thrashr888/agentkernel/releases
3. **npm**: https://www.npmjs.com/package/agentkernel
4. **PyPI**: https://pypi.org/project/agentkernel-sdk/
5. **crates.io**: https://crates.io/crates/agentkernel-sdk
6. **Homebrew**: `brew info thrashr888/agentkernel/agentkernel`

For the desktop app, also verify that the published DMG is stapled and that
the installed app passes `codesign --verify --deep --strict` and Gatekeeper’s
assessment. The first fixed release must be a new patch version such as
`v0.19.1`; `v0.19.0` remains immutable under the roll-forward policy.

## Troubleshooting

**Registry already has this version (crates.io, npm, PyPI)**: Package registries are immutable — once a version is published, it cannot be overwritten or deleted. If the tag was pushed and some registries succeeded before a failure, re-tagging will not fix it. The publish workflows have idempotency checks that skip already-published versions, so re-runs are safe. But the correct fix is always to roll forward to a new patch version.

**SDK publish fails but Release succeeds**: The workflows are independent. Fix the failing SDK job and re-run it from the Actions tab — no need to retag.

**npm dual-publish fails on GitHub Packages**: The GitHub Packages step temporarily rewrites `package.json` to use the scoped name `@thrashr888/agentkernel`. If it fails, the original `package.json` is restored by `git checkout`. The npm publish (unscoped) is unaffected.

**PyPI OIDC fails**: Verify the trusted publisher config matches exactly: repo owner, repo name, workflow filename, and environment name. The `pypi` environment must exist in GitHub repo settings.

**npm OIDC fails**: Verify the trusted publisher config on npmjs.com matches exactly: username, repo name, workflow filename (`sdk-publish.yml`), and environment name (`npm`). The `npm` environment must exist in GitHub repo settings. Requires npm >= 11.5.1.

**crates.io OIDC fails**: Verify the trusted publisher config on crates.io matches exactly: username, repo name, workflow filename (`sdk-publish.yml`), and environment name (`crates`). The `crates` environment must exist in GitHub repo settings. Uses `rust-lang/crates-io-auth-action@v1`.

**crates.io publish fails**: `cargo publish` requires that `Cargo.toml` metadata is complete (description, license, repository). The SDK's `Cargo.toml` already has these fields.

**Version mismatch**: SDK versions are injected from the Git tag. The `Cargo.toml` in the repo root (for the CLI) is the only version you update manually.
