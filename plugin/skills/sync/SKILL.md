---
name: sync
description: Bring this project's agent-memory entries up to date with what happened in the current session. Updates the project-state entry and saves new techniques, gotchas and decisions. Use before finishing a substantial piece of work, before context compaction, or when the user asks to save, sync or update memory.
---

# Sync this session into agent-memory

Follow the conventions in the agent-memory **guide** skill. The project key is the one printed by the SessionStart hook, or worked out from `git remote get-url origin`.

1. **Load the current state.** `search` with `project: <key>` and `tags: ["project-state"]`, then `get` the entry. If none exists, run agent-memory **bootstrap** instead and stop here.

2. **Work out what changed.** Review this session: features added, decisions made (and why), commands or setup that changed, deploy changes, problems found or fixed, and what's in progress or next. `git log` and `git status` help, but the reasons usually only exist in the conversation.

3. **Update the project-state entry** with `edit`: quote the lines that changed as `old_text` and give their replacements, rather than resending the whole body. Rewrite the sections whose facts changed, especially **Current status**, so the entry reads as current truth, not a changelog. Remove anything that is no longer true.

4. **Save standalone knowledge** as separate entries: a fix someone could hit again, a technique worth reusing, or a gotcha. `search` first, and `edit` a matching entry rather than adding a near-duplicate. Add any new entries to the project-state **Related entries** section.

5. **Report briefly**: one or two lines on what you updated or added. If nothing meaningful changed, say so and make no writes.
