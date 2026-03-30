#!/usr/bin/env sh

set -eu

# Linked git worktrees should share the main checkout's .beads runtime state.
# bd understands a local .beads/redirect file that points at the canonical
# .beads directory, so create it on checkout if this is a linked worktree.

git_dir=$(git rev-parse --git-dir 2>/dev/null || exit 0)
common_dir=$(git rev-parse --git-common-dir 2>/dev/null || exit 0)

if [ "$git_dir" = "$common_dir" ]; then
  exit 0
fi

worktree_root=$(git rev-parse --show-toplevel 2>/dev/null || exit 0)
main_root=$(cd "$common_dir/.." 2>/dev/null && pwd -P) || exit 0
main_beads="$main_root/.beads"

if [ ! -d "$main_beads" ]; then
  exit 0
fi

mkdir -p "$worktree_root/.beads"
redirect_path="$worktree_root/.beads/redirect"

current_target=""
if [ -f "$redirect_path" ]; then
  current_target=$(cat "$redirect_path" 2>/dev/null || true)
fi

if [ "$current_target" != "$main_beads" ]; then
  printf '%s\n' "$main_beads" > "$redirect_path"
fi
