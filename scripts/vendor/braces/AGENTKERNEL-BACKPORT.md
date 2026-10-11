# AgentKernel security backport of braces 3.0.3

This directory is a maintained, security-only vendored copy of the published
`braces@3.0.3` package, not an upstream patched release. Its original package
name, version, author, repository, README and MIT license are preserved. The
package's `files` list additionally includes this provenance document.

## Provenance

- Published source: <https://registry.npmjs.org/braces/-/braces-3.0.3.tgz>
- npm integrity:
  `sha512-yQbXgO/OSZVD2IsiLlro+7Hf6Q18EJrKSEsdoMzKePKXct3gvD8oLcOQdIzGupr5Fj+EDe8gO/lxc1BzfMpxvA==`
- npm SHA-1: `490332f40919452272d55a8480adc0c441358789`
- Security reference: <https://github.com/micromatch/braces/pull/72>, closed and
  unreleased when this backport was prepared on 2026-10-11
- Immutable reference:
  <https://github.com/micromatch/braces/commit/28d440b5dd449dbf1fe6f3506cf94ecca4d02660>
- Advisory: <https://github.com/advisories/GHSA-vfj7-8cjw-p6xm>

The changes are limited to `lib/constants.js`, `lib/parse.js`, `lib/compile.js`,
`lib/expand.js` and `lib/stringify.js`:

- Set a hard ceiling of 100 combined brace/parenthesis nesting levels before
  allocating another nested parser node
- Allow a stricter finite numeric `maxDepth` option, checking `nesting + 1`
  so fractional limits cannot admit an extra level; NaN, Infinity and larger
  limits cannot disable the hard ceiling
- Bound the recursive compile, expand and stringify walkers, including direct
  AST inputs and cyclic child graphs, with explicit depth errors; expand's
  fallback stringification shares the remaining traversal depth budget
- Detect cycles in expand's parent-chain traversal rather than hanging

The security-only parser diff intentionally excludes the reference branch's
unrelated unmatched-quote handling and comma invalidation changes. Stringify
continues omitting the recursive parent argument to preserve `escapeInvalid`
behavior. Existing range/output-size semantics are unchanged; this backport
addresses nesting stack exhaustion, not every possible resource-exhaustion
pattern or hostile JavaScript object accessor.

The remaining-budget propagation in expand is an additional security-only
tightening beyond the reference commit, so invalid/dollar subtrees cannot reset
the direct-AST depth counter when they are stringified.

## Installation and verification

The bridge declares `braces: file:vendor/braces` and the `$braces` npm override,
so micromatch uses this exact local package. `scripts/.npmrc` sets
`install-links=true`, packaging the local dependency instead of installing its
old development dependencies through a directory link. Run npm from `scripts`
(or use `npm --prefix scripts`) so this project configuration is applied.
Keep `scripts/vendor/braces` with the bridge when distributing the scripts.

The lockfile resolves braces to `file:vendor/braces`; it does not pretend that
the registry tarball contains this fix. Security tests compare all installed
runtime files, package metadata and license against this source, and verify
the Daytona → fast-glob → micromatch → braces resolution path. Do not reuse a
cached upstream `braces@3.0.3` installation without running `npm ci`.

From `scripts`, run `npm ci --no-audit` and `npm test`. The test suite includes
the security boundaries, direct AST/cycle cases, string compatibility,
micromatch/fast-glob consumers and provider contracts. It also executes all
12 unmodified upstream 3.0.3 test files (764 assertions registered as tests)
through a small Node test-runner adapter. The adapter replaces only Mocha's
`describe`/`it` harness, resolves the package imports to this backport and
resolves Bash through PATH. Bash must be available. No legacy upstream
development dependencies are installed.

Upstream fixtures are in `scripts/test/fixtures/braces-upstream`, under the
MIT license in this directory. Their source archive is
<https://codeload.github.com/micromatch/braces/tar.gz/refs/tags/3.0.3>, SHA-256
`2d4ffb09194737a619975e848dcc32d12b3570da05866baa94e16fa0b3863eab`.

## Scanner status and maintenance

Version-based scanners can continue reporting this dependency because its
truthful package identity remains `braces@3.0.3`, an affected upstream version.
The source change and regression tests establish the nesting mitigation; a
clean version-only audit is neither claimed nor manufactured. This backport
does not dismiss or suppress any Dependabot alert. Review any finding against
the vendored source and this provenance.

AgentKernel owns maintenance of this copy until a reviewed, compatible
upstream release includes the fix. When replacing the backport, remove its
direct dependency, override and installation setting together, regenerate the
lockfile, verify the actual installed dependency path, and rerun the upstream,
security and provider tests. Reassess the scanner status using the resulting
dependency graph.
