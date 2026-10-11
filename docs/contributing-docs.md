# Contributing to the docs

The documentation site uses MkDocs Material. From a repository checkout:

```bash
python3 -m venv .venv-docs
. .venv-docs/bin/activate
python -m pip install -r requirements-docs.txt
mkdocs build --strict
mkdocs serve
```

Keep the local virtual environment and generated `site/` output out of commits.
The pinned requirements match the documentation CI environment. Pull requests
build the site with strict validation; only main-branch runs deploy to GitHub
Pages.

## Links and navigation

Use relative links to Markdown files inside `docs/`, including fragment IDs
when linking to a section. Files outside the docs directory need a full
repository URL. For example, link to an example under
`https://github.com/thrashr888/agentkernel/tree/main/examples/` rather than a
relative path that escapes `docs/`.

Add new pages to `mkdocs.yml`. Strict validation checks missing navigation
entries, missing files, unrecognized relative links, and missing anchors.
External HTTP destinations still need a separate check.

## Keep instructions accurate

Verify command flags against the current CLI and examples against the code
that handles them. CLI, HTTP, and MCP defaults may differ: for example, the CLI
uses `--fast` as an opt-in switch, while HTTP `/run` and MCP `sandbox_run`
default to the container pool.

Describe backend requirements and unsupported operations explicitly. Keep
filesystem snapshots separate from full-state checkpoints, and preserve
preview labels until their native validation gates pass.

For performance claims, include the workload, host, software versions, timing
boundary, and cold/warm state. Pool acquisition, guest boot, command execution,
and full lifecycle measurements are different metrics. Link to a reproducible
report before claiming a speedup across backends.
