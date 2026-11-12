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
| `--mongodb-uri` | `MONGODB_URI` | `mongodb://127.0.0.1:27017` |
| `--database` | `MONGODB_DATABASE` | `agent_memory` |
| `--bind` | `BIND_ADDR` | `0.0.0.0:8787` |
| `--allowed-host` (repeatable, or comma-separated) | `ALLOWED_HOSTS` | loopback only |
| `--allow-any-host` | `ALLOW_ANY_HOST` | off |

The MCP endpoint is `/mcp` and `/healthz` returns `ok`. Set `RUST_LOG=debug` for more logging.

**Host check:** rmcp rejects requests whose `Host` header isn't on the allow-list. This protects against DNS rebinding. Loopback names are always allowed. Add every name clients use to reach the server: the Tailscale MagicDNS name, the short host name and the tailnet IP.

## Testing

```bash
cargo test
```

Unit tests always run. The end-to-end tests in `tests/mcp_integration.rs` start the real HTTP server and call every tool with rmcp's MCP client. They run only when `MONGODB_TEST_URI` is set. Each test creates and then drops its own `agent_memory_test_<id>` database:

```bash
docker run -d --name amm-test-mongo -p 127.0.0.1:27018:27017 mongo:7
MONGODB_TEST_URI=mongodb://127.0.0.1:27018 cargo test
```

MongoDB 8 and 9 refuse to start on Linux kernels 6.19 through 7.0.13 ([SERVER-121912](https://jira.mongodb.org/browse/SERVER-121912)), which includes current Docker Desktop VMs. That's why the local example uses `mongo:7`. CI runs against `mongo:9`.

## Debian package

The package is built with the standard Debian tooling (`debhelper`, `dpkg-buildpackage`, `lintian`) inside a trixie container:

```bash
docker build --platform linux/amd64 -f packaging/Dockerfile --output dist .
```

This produces `dist/agent-memory-mcp_<version>_amd64.deb` (plus a `-dbgsym` package). Trixie's own Rust (1.85) is too old for rmcp, so the build image installs a pinned toolchain with rustup. The package installs:

| Path | What |
|------|------|
| `/usr/sbin/agent-memory-mcp` | the server |
| `/usr/lib/systemd/system/agent-memory-mcp.service` | systemd unit, enabled and started on install |
| `/etc/default/agent-memory-mcp` | configuration (a conffile, so your edits survive upgrades) |
| `agent-memory-mcp(8)` | man page |

The service runs as a transient `DynamicUser` with a locked-down sandbox. It keeps no local state.

To deploy on fuzzball:

```bash
sudo apt install ./agent-memory-mcp_0.1.0_amd64.deb
sudoedit /etc/default/agent-memory-mcp   # set ALLOWED_HOSTS to the tailnet name(s)
sudo systemctl restart agent-memory-mcp
journalctl -u agent-memory-mcp -f
```

To release a new version, add an entry with `dch -v <version>` (or edit `debian/changelog`) and bump `version` in `Cargo.toml` to match.

## CI

`.github/workflows/ci.yml` runs in a `debian:trixie` container on every push to `main` and on pull requests. It checks formatting, runs clippy, runs the full test suite against a `mongo:9` service container, builds the package with `dpkg-buildpackage`, lints it with lintian, and uploads the `.deb` as a job artifact.

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

## License

MIT. See [LICENSE](LICENSE).
