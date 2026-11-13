#!/usr/bin/env bash
# Fail unless every version in the repo matches debian/changelog.
#
# debian/changelog is the source of truth, because its version is what apt
# deploys. Cargo.toml (and so `agent-memory-mcp --version`) and both plugin
# manifests must match it. Release with:
#   dch -v X.Y.Z   then set the same version in the files listed below.
#
# Run from anywhere: `scripts/check-versions.sh`. debian/rules and CI run it.

set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

# Top entry of the changelog, e.g. "agent-memory-mcp (0.2.0) trixie; ...".
# Strip any epoch ("1:") and Debian revision ("-1") to get the upstream version.
deb_version=$(sed -n '1s/^[^(]*(\([^)]*\)).*/\1/p' "$root/debian/changelog")
expected=${deb_version#*:}
expected=${expected%-*}

if [ -z "$expected" ]; then
  echo "check-versions: could not read a version from debian/changelog" >&2
  exit 1
fi

status=0
check() {
  local file=$1 found=$2
  if [ "$found" = "$expected" ]; then
    printf '  ok        %-36s %s\n' "$file" "$found"
  else
    printf '  MISMATCH  %-36s %s (expected %s)\n' "$file" "${found:-<none>}" "$expected"
    status=1
  fi
}

# First `version = "..."` line, which is the [package] one.
cargo_version=$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "$root/Cargo.toml" | head -n1)
json_version() {
  sed -n 's/^[[:space:]]*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$1" | head -n1
}

echo "debian/changelog version: $deb_version (upstream $expected)"
check Cargo.toml "$cargo_version"
check plugin/.claude-plugin/plugin.json "$(json_version "$root/plugin/.claude-plugin/plugin.json")"
check plugin/.codex-plugin/plugin.json "$(json_version "$root/plugin/.codex-plugin/plugin.json")"

if [ "$status" -ne 0 ]; then
  echo "check-versions: versions differ from debian/changelog; update them to $expected" >&2
fi
exit "$status"
