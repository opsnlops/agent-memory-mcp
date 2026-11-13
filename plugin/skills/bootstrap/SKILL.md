---
name: bootstrap
description: Seed agent-memory for a project that has no project-state entry yet (one that predates agent-memory). Collects what is already known about the project and saves it as a project-state entry plus separate technique and gotcha entries. Use when a search finds no project-state for the current project, or when the user asks to bootstrap, import or seed memory.
argument-hint: "[project key override]"
---

# Bootstrap agent-memory for this project

Follow the conventions in the agent-memory **guide** skill. The project key is the one printed by the SessionStart hook (or worked out from `git remote get-url origin`), unless the user gave a different one. In Claude Code, it arrives here: `$ARGUMENTS`.

## 1. Make sure it's needed

`search` with `project: <key>` and `tags: ["project-state"]`. If an entry already exists, stop bootstrapping. Run agent-memory **sync** instead to bring it up to date.

Also `search` with `project: <key>` and no tags. Earlier sessions may have saved loose entries. Fold those into the plan below instead of duplicating them.

## 2. Gather what's known

Read, don't guess. Use whichever of these exist:

- **Repo docs**: `README*`, `CLAUDE.md`, `AGENTS.md`, `CONTRIBUTING*`, `docs/`, and any `*intent*.md` or design notes.
- **Build and run setup**: package manifests (`Cargo.toml`, `package.json`, `pyproject.toml`, `go.mod`...), `Makefile`/`justfile`, `Dockerfile`, CI workflows, deploy files (`debian/`, systemd units, k8s manifests, compose files).
- **History**: `git log --oneline -30` and the remote URL.
- **Local agent memory for this project**: Claude Code's file-based memory, usually `~/.claude/projects/<encoded-project-path>/memory/` with `MEMORY.md` as the index. Codex: any saved memories you can see about this project, and the global `~/.codex/AGENTS.md`.
- **The current conversation**: anything the user told you, or that you found, that isn't written down.

Keep it to a quick survey. Skim, then write; don't read the whole codebase.

## 3. Write the entries

1. **One `project-state` entry**: title `Project state: <key>`, kind `memory`, tags `["project-state", ...2-4 topic tags]`, `project: <key>`, `author` set. Use the body sections from the guide skill. Mark anything you inferred rather than confirmed as "(unverified)".
2. **Separate entries** for non-obvious knowledge that stands on its own: setup gotchas, workarounds, debugging techniques and decisions with their reasons. Use kind `technique` or `memory`, tagged, with `project: <key>`, or no project if it applies everywhere. Search first and `edit` any match.
3. **Local memory**: copy anything still useful from the agent's local or built-in memory for this project into agent-memory. Leave the local copies alone unless the user asks you to remove them.
4. Go back and list the new entries' titles and ids in the project-state entry's **Related entries** section.

Never save secrets, credentials or tokens, even when local memory or config files contain them.

## 4. Report

Tell the user in a few lines what you created: the project-state entry plus how many other entries, and anything you marked unverified that they could confirm.
