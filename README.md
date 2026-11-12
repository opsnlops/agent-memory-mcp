# agent-memory-mcp

A shared long-term memory for coding agents. One session can save a memory or technique, and any other session can find it, whether it's Claude Code, Codex, or another MCP client. The server speaks MCP over Streamable HTTP and stores everything in MongoDB.

There is no authentication. Access control is left to the network (for example Tailscale).

## Tools

| Tool | What it does |
|------|--------------|
| `remember` | Save a new entry: `title`, `body`, plus optional `kind` (`memory` or `technique`), `tags`, `project` and `author`. |
| `search` | Free-text search over title, tags and body, optionally filtered by `tags` (all must match), `kind` and `project`. Without a query it lists the most recent entries. Returns snippets. |
| `get` | Fetch one entry by id, with its full body. |
| `update` | Change selected fields of an entry. |
| `forget` | Delete an entry. |
| `list_tags` | List tags in use with their counts. |

Tags are lowercased, trimmed and deduplicated. Text search uses a weighted MongoDB text index (title 10, tags 5, body 1), and the server creates its indexes on startup.

## Running

```bash
cargo build --release
./target/release/agent-memory-mcp --allowed-host my-box.tailnet-name.ts.net
```

| Flag | Env | Default |
|------|-----|---------|
| `--mongodb-uri` | `MONGODB_URI` | `mongodb://fuzzball.local:27017` |
| `--database` | `MONGODB_DATABASE` | `agent_memory` |
| `--bind` | `BIND_ADDR` | `0.0.0.0:8787` |
| `--allowed-host` (repeatable, or comma-separated) | `ALLOWED_HOSTS` | loopback only |
| `--allow-any-host` | `ALLOW_ANY_HOST` | off |

The MCP endpoint is `/mcp` and `/healthz` returns `ok`. Set `RUST_LOG=debug` for more logging.

**Host check:** rmcp rejects requests whose `Host` header isn't on the allow-list. This protects against DNS rebinding. Loopback names are always allowed. Add every name clients use to reach the server: the Tailscale MagicDNS name, the short host name and the tailnet IP.

## Connecting agents

Claude Code (user scope, so every project gets it):

```bash
claude mcp add --transport http --scope user agent-memory http://my-box.tailnet-name.ts.net:8787/mcp
```

Codex (`~/.codex/config.toml`):

```toml
[mcp_servers.agent-memory]
url = "http://my-box.tailnet-name.ts.net:8787/mcp"
```

To get agents using it without being asked, add a line like this to your global `CLAUDE.md` / `AGENTS.md`:

> Before non-trivial work, search the agent-memory MCP for relevant memories and techniques. When you learn something a future session would benefit from, `remember` it with a clear title and tags.
