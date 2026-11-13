---
name: guide
description: Conventions for the shared agent-memory MCP server. Use before starting work (to find prior art and the project's state), whenever you learn something another session could reuse, when deciding where to save a memory, and when a project's state changes.
---

# agent-memory conventions

agent-memory is a shared memory server used by every Claude Code and Codex session. Treat it as your **primary memory**. Anything you save there, other agents can find and act on, including agents of a different kind.

Its tools are `search`, `get`, `remember`, `edit`, `update`, `forget` and `list_tags`. The full tool names depend on the client: in Claude Code, `mcp__plugin_agent-memory_agent-memory__<tool>` or `mcp__agent-memory__<tool>`; in Codex, the tools of the `agent-memory` MCP server. They all reach the same server.

## Changing an entry

- **Use `edit` for any change to an existing body.** Pass `edits`, each an exact `old_text` copied from the body and its `new_text`, and/or `append` to add text at the end. Don't resend the whole body with `update`: on a long entry that costs thousands of tokens each way, and it overwrites whatever another session wrote in the meantime. `edit` is atomic and re-applies your edits if someone else wrote first.
- `old_text` must match exactly once, including whitespace and line breaks. Quote a whole line or two for uniqueness. If it fails, the error says which edit and why, and nothing is saved.
- **Read only what you need.** `get` lists the body's `headings`, and `get` with `section: "<heading>"` returns one section of a long entry.
- Use `update` for the title, tags, kind or project, or to replace a short body wholesale.
- Writes return the entry's metadata and `body_chars`, not the body. `get` it if you need to check the result.

## Where memories go

- **agent-memory first.** Project context, decisions, techniques, gotchas and user preferences all go here.
- **Built-in or local memory:** Claude Code's file-based memory (`MEMORY.md`) or Codex's saved memories. Use it only when agent-memory is unreachable. Mark those notes "pending sync to agent-memory", copy them over once the server is back, then delete the local copies.
- **Never store** secrets, credentials, tokens, private keys, or personal data beyond a name. agent-memory has no authentication and every agent can read it.
- **Don't store** what the repo already records well: the code itself, git history, or anything that is obvious from reading the files. Store the non-obvious parts: why things are the way they are, how to work with the project, and what went wrong before.

## Project key

Pass `project` on every project-specific entry. The key is the bare repo name from the `origin` remote, lowercased (for example `agent-memory-mcp`). Without a remote, use the git top-level directory name, then the working directory name. The plugin's SessionStart hook prints the key for the current directory. If the hook didn't run (Codex requires approving plugin hooks with `/hooks` first), work out the key yourself, for example with `git remote get-url origin`. Leave `project` empty for knowledge that applies everywhere.

## Before starting work

1. `search` with `project: <key>` and `tags: ["project-state"]`, then `get` the result to read the full project state.
2. `search` for the task itself, with free-text keywords, both with `project: <key>` and without it. Techniques from other projects often apply.
3. If no `project-state` entry exists, the project predates agent-memory. Run the agent-memory **bootstrap** skill.

Search uses whole-word text matching with stemming: no partial words and no typo tolerance. Try two or three phrasings, and use `list_tags` to discover topic names.

## The project-state entry

Each project has exactly one entry tagged `project-state`, titled `Project state: <key>`, with kind `memory`. It is the starting point for any agent arriving at the project. Keep it current with `edit`, never by adding a second one. Body sections, all in Markdown:

- **Purpose**: what the project is and who uses it.
- **Stack**: languages, frameworks, key dependencies and their versions where it matters.
- **Layout**: the important directories and entry points.
- **Build, test, run**: the exact commands, plus any required services or environment variables.
- **Deployment**: where it runs, how it gets there, and where its config lives.
- **Current status**: what works, what is in progress, what is next, and known problems.
- **Decisions**: notable choices and the reasons behind them.
- **Related entries**: titles and ids of technique or gotcha entries for this project.

Update it when work changes something it says: a feature lands, a decision is made, the deploy target changes, or a bug becomes known. For a long session, use the agent-memory **sync** skill before finishing.

## Other entries

- **kind `technique`** for reusable how-tos: commands, recipes, workarounds, debugging approaches.
- **kind `memory`** for facts, decisions, context and gotchas.
- **Title**: a specific, searchable statement of the point, such as "MongoDB 8/9 Docker images won't start on Docker Desktop (kernel 6.19–7.0.13)". Avoid vague titles like "Docker issue".
- **Body**: enough context that an agent with no history can act on it. Include the symptom, the cause, the fix, and the exact commands or error text.
- **Tags**: 2–6 lowercase topic tags. Run `list_tags` first and reuse existing tags.
- **author**: your agent name: `claude-code` or `codex`.
- **Search before saving.** If a matching entry exists, `edit` it instead of adding a near-duplicate. If an entry turns out to be wrong, fix it with `edit`, or `forget` it when it is obsolete.

## Telling the user

Briefly mention memory operations that affect the user's understanding, such as "Found prior notes on this project's deploy setup" or "Saved the fix as an agent-memory technique". Don't narrate routine searches.
