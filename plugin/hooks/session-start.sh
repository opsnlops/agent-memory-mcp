#!/usr/bin/env bash
# SessionStart hook for the agent-memory plugin, shared by Claude Code
# (hooks/hooks.json) and Codex (hooks/codex-hooks.json).
#
# Prints plain text, which both agents add to the session context. Never
# fails the session: every probe is best-effort and the script always exits 0.

set -u

input=$(cat 2>/dev/null || true)

# Pull a top-level string field out of the hook's stdin JSON without jq.
json_field() {
  printf '%s' "$input" | sed -n "s/.*\"$1\"[[:space:]]*:[[:space:]]*\"\([^\"]*\)\".*/\1/p" | head -n1
}

# "startup", "resume", "clear", "compact" or "fork".
source_kind=$(json_field source)
source_kind=${source_kind:-startup}

# Claude Code sets CLAUDE_PROJECT_DIR; Codex passes the session cwd on stdin.
stdin_cwd=$(json_field cwd)
project_dir=${CLAUDE_PROJECT_DIR:-${stdin_cwd:-$PWD}}

# Project key: bare repo name from the origin remote, else the git top-level
# directory name, else the working directory name. Lowercased so every agent
# derives the same key.
project_key=""
if remote=$(git -C "$project_dir" remote get-url origin 2>/dev/null) && [ -n "$remote" ]; then
  project_key=$(basename "${remote%/}")
  project_key=${project_key%.git}
elif top=$(git -C "$project_dir" rev-parse --show-toplevel 2>/dev/null) && [ -n "$top" ]; then
  project_key=$(basename "$top")
else
  project_key=$(basename "$project_dir")
fi
project_key=$(printf '%s' "$project_key" | tr '[:upper:]' '[:lower:]')

# Reachability: hit /healthz next to the configured /mcp endpoint. Claude Code
# exports the plugin option; Codex has no plugin options, so AGENT_MEMORY_URL
# can override the default there.
server_url=${CLAUDE_PLUGIN_OPTION_SERVER_URL:-${AGENT_MEMORY_URL:-http://10.19.63.10:8787/mcp}}
health_url="${server_url%/mcp}/healthz"
reachable=unknown
if command -v curl >/dev/null 2>&1; then
  if [ "$(curl -s -m 3 "$health_url" 2>/dev/null)" = "ok" ]; then
    reachable=yes
  else
    reachable=no
  fi
fi

echo "## agent-memory"
echo
echo "The agent-memory MCP server is this agent's primary, shared memory: other Claude Code and Codex sessions read and write the same store."
echo "Project key for this directory: \`${project_key}\` (pass it as \`project\` when searching or saving project-specific entries)."

if [ "$reachable" = "no" ]; then
  echo
  echo "WARNING: the agent-memory server at ${server_url} did not answer its health check. Until it is back, keep notes in local memory, mark them \"pending sync to agent-memory\", and copy them over (then delete the local copies) once the server responds."
fi

case "$source_kind" in
  resume)
    echo
    echo "Session resumed. If the project's state changed since you last saved it, update its \`project-state\` entry before you finish."
    ;;
  compact)
    echo
    echo "Context was just compacted. Before continuing, re-read this project's \`project-state\` entry (search with project \`${project_key}\` and tag \`project-state\`) and save anything you learned before the compaction that isn't stored yet."
    ;;
  *)
    echo
    echo "Before starting non-trivial work:"
    echo "1. Search agent-memory for this project's state: \`search\` with project \`${project_key}\` and tags [\"project-state\"], then \`get\` the entry for its full body."
    echo "2. Search for prior art related to the task, both within the project and across all projects (omit \`project\`): techniques, gotchas and decisions."
    echo "3. If there is no \`project-state\` entry for \`${project_key}\`, the project predates agent-memory: run the agent-memory \`bootstrap\` skill before or alongside the user's task, and tell the user you did."
    echo
    echo "Keep it current: when work changes what is true about the project, update its \`project-state\` entry; save reusable fixes and techniques as their own entries. Prefer agent-memory over your built-in or local memory (memory files, saved memories). The agent-memory \`guide\` skill has the conventions."
    ;;
esac

exit 0
