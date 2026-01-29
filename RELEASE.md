# Release Process

## Overview

Pushing a `v*` tag triggers two workflows in parallel:

| Workflow | File | What it does |
|----------|------|--------------|
| **Release** | `.github/workflows/release.yml` | Builds CLI binaries for 4 platforms, creates GitHub Release |
| **SDK Publish** | `.github/workflows/sdk-publish.yml` | Publishes all SDKs to their registries |

## Cutting a Release

```bash
# 1. Update version in Cargo.toml (CLI)
#    SDK versions are set automatically from the tag.

# 2. Commit
git add Cargo.toml
git commit -m "release: v0.3.0"

# 3. Tag and push
git tag v0.3.0
git push origin main v0.3.0
```

That's it. Both workflows trigger on the tag push.

## What Gets Published

### CLI Binaries (release.yml)

Cross-compiled on native runners, uploaded to GitHub Releases as `.tar.gz`:

| Platform | Runner | Artifact |
|----------|--------|----------|
| Linux x64 | `ubuntu-latest` | `agentkernel-linux-x64.tar.gz` |
| Linux arm64 | `ubuntu-24.04-arm` | `agentkernel-linux-arm64.tar.gz` |
| macOS arm64 | `macos-latest` | `agentkernel-darwin-arm64.tar.gz` |
| macOS x64 | `macos-13` | `agentkernel-darwin-x64.tar.gz` |

### SDKs (sdk-publish.yml)

All 4 jobs run in parallel:

| SDK | Registry | Package Name | Auth |
|-----|----------|-------------|------|
| Node.js | npmjs.com | `agentkernel` | `NPM_TOKEN` secret |
| Node.js | GitHub Packages | `@thrashr888/agentkernel` | `GITHUB_TOKEN` (automatic) |
| Python | PyPI | `agentkernel` | OIDC trusted publisher (no secret) |
| Rust | crates.io | `agentkernel-sdk` | `CARGO_REGISTRY_TOKEN` secret |
| Swift | Git tags (no registry) | `AgentKernel` | None (verified only) |

Version is extracted from the tag name (`v0.3.0` → `0.3.0`) and injected into each SDK's manifest before publishing.

### Homebrew (manual)

After the GitHub Release is created:

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
brew tap thrashr888/agentkernel
brew install agentkernel
# or: brew upgrade agentkernel
```

## One-Time Setup (per registry)

These secrets and configurations must be set up once before the first publish.

### npm (`NPM_TOKEN`)

1. Go to https://www.npmjs.com/settings/tokens/granular-access-tokens/new
2. Create a token with publish access to `agentkernel`
3. Add as repo secret: Settings → Secrets → Actions → `NPM_TOKEN`

### PyPI (OIDC trusted publisher)

1. Create the project at https://pypi.org (first publish may need a manual upload)
2. Go to project settings → Publishing → Add a new publisher:
   - Owner: `thrashr888`
   - Repository: `agentkernel`
   - Workflow: `sdk-publish.yml`
   - Environment: `pypi`
3. Create a deployment environment in GitHub: Settings → Environments → `pypi`

### crates.io (`CARGO_REGISTRY_TOKEN`)

1. Go to https://crates.io/settings/tokens/new
2. Create a token with publish access
3. Add as repo secret: Settings → Secrets → Actions → `CARGO_REGISTRY_TOKEN`

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
4. **PyPI**: https://pypi.org/project/agentkernel/
5. **crates.io**: https://crates.io/crates/agentkernel-sdk
6. **Homebrew**: `brew info thrashr888/agentkernel/agentkernel`

## Troubleshooting

**SDK publish fails but Release succeeds**: The workflows are independent. Fix the failing SDK job and re-run it from the Actions tab — no need to retag.

**npm dual-publish fails on GitHub Packages**: The GitHub Packages step temporarily rewrites `package.json` to use the scoped name `@thrashr888/agentkernel`. If it fails, the original `package.json` is restored by `git checkout`. The npm publish (unscoped) is unaffected.

**PyPI OIDC fails**: Verify the trusted publisher config matches exactly: repo owner, repo name, workflow filename, and environment name. The `pypi` environment must exist in GitHub repo settings.

**crates.io publish fails**: `cargo publish` requires that `Cargo.toml` metadata is complete (description, license, repository). The SDK's `Cargo.toml` already has these fields.

**Version mismatch**: SDK versions are injected from the Git tag. The `Cargo.toml` in the repo root (for the CLI) is the only version you update manually.
