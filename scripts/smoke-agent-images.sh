#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="$repo_root/examples/agents/tested-versions.json"
build=true

if [[ "${1:-}" == "--no-build" ]]; then
  build=false
  shift
fi

if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required to read $manifest" >&2
  exit 1
fi

if ! command -v docker >/dev/null 2>&1; then
  echo "docker is required to build and smoke agent images" >&2
  exit 1
fi

if [[ "$#" -gt 0 ]]; then
  agent_ids=$(printf '%s\n' "$@")
else
  agent_ids=$(jq -r '.agents[].id' "$manifest")
fi

while IFS= read -r agent_id; do
  [[ -n "$agent_id" ]] || continue

  if ! jq -e --arg id "$agent_id" '.agents[] | select(.id == $id)' "$manifest" >/dev/null; then
    echo "unknown agent: $agent_id" >&2
    exit 1
  fi

  version=$(jq -r --arg id "$agent_id" '.agents[] | select(.id == $id) | .version' "$manifest")
  executable=$(jq -r --arg id "$agent_id" '.agents[] | select(.id == $id) | .executable' "$manifest")
  smoke_arg=$(jq -r --arg id "$agent_id" '.agents[] | select(.id == $id) | .smoke_arg' "$manifest")
  expected_exit=$(jq -r --arg id "$agent_id" '.agents[] | select(.id == $id) | .expected_exit // 0' "$manifest")
  expected_output=$(jq -r --arg id "$agent_id" '.agents[] | select(.id == $id) | .expected_output' "$manifest")
  base=$(jq -r --arg id "$agent_id" '.agents[] | select(.id == $id) | .base' "$manifest")
  runtime_user=$(jq -r '.defaults.runtime_user' "$manifest")
  workspace=$(jq -r '.defaults.workspace' "$manifest")
  node_major=$(jq -r --arg base "$base" '.base_images[$base].node_major // ""' "$manifest")
  alpine_major_minor=$(jq -r --arg base "$base" '.base_images[$base].alpine_major_minor // ""' "$manifest")
  image="agentkernel-smoke/${agent_id}:${version}"
  context="$repo_root/examples/agents/$agent_id"

  if [[ "$build" == true ]]; then
    docker build --pull --tag "$image" "$context"
  fi

  docker run --rm --entrypoint /bin/sh "$image" -lc '
    test "$(id -un)" = "$1"
    test "$PWD" = "$2"
    test -w "$2"
    command -v "$3" >/dev/null
    test "$(node -p "process.versions.node.split(\".\")[0]")" = "$4"
    if [ -n "$5" ]; then
      test "$(cut -d. -f1,2 /etc/alpine-release)" = "$5"
    fi
    probe="$2/.agentkernel-smoke-write"
    : > "$probe"
    rm "$probe"
  ' _ "$runtime_user" "$workspace" "$executable" "$node_major" "$alpine_major_minor"

  set +e
  smoke_output=$(docker run --rm "$image" "$executable" "$smoke_arg" 2>&1)
  smoke_exit=$?
  set -e
  if [[ "$smoke_exit" -ne "$expected_exit" ]]; then
    echo "$agent_id smoke exited $smoke_exit; expected $expected_exit" >&2
    echo "$smoke_output" >&2
    exit 1
  fi
  if [[ "$smoke_output" != *"$expected_output"* ]]; then
    echo "$agent_id smoke output did not contain expected text: $expected_output" >&2
    echo "$smoke_output" >&2
    exit 1
  fi
  echo "$agent_id $version: ok"
done <<< "$agent_ids"
