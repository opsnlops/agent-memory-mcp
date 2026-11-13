# agent-memory-mcp

A shared long-term memory for coding agents. One session can save a memory or technique, and any other session can find it, whether it's Claude Code, Codex, or another MCP client. The server speaks MCP over Streamable HTTP and stores everything in MongoDB.

There is no authentication. Access control is left to the network (for example Tailscale).

## Tools

| Tool | What it does |
|------|--------------|
| `remember` | Save a new entry: `title`, `body`, plus optional `kind` (`memory` or `technique`), `tags`, `project` and `author`. |
| `search` | Free-text search over title, tags and body, optionally filtered by `tags` (all must match), `kind` and `project`. Without a query it lists the most recent entries. Returns snippets. |
| `get` | Fetch one entry by id, with its full body and a list of its Markdown headings. Pass `section` (a heading) to get just that part of a long entry. |
| `edit` | Change part of a body without resending it: exact-text replacements (`old_text` → `new_text`, optionally `replace_all`) and/or `append`. Atomic and all-or-nothing. If another session writes first, the edits are re-applied to its version (a compare-and-swap on the body), so concurrent edits don't overwrite each other. |
| `update` | Change the title, kind, tags or project, or replace the whole body. |
| `forget` | Delete an entry. |
| `list_tags` | List tags in use with their counts. |

Writes (`remember`, `edit`, `update`) return the entry's metadata and `body_chars` rather than echoing the body. Tags are lowercased, trimmed and deduplicated. Text search uses a weighted MongoDB text index (title 10, tags 5, body 1), and the server creates its indexes on startup.

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
| `/etc/default/agent-memory-mcp` | configuration (a conffile, so your edits survive upgrades; mode 0600 because `MONGODB_URI` can carry credentials) |
| `agent-memory-mcp(8)` | man page: every option, plus configuration, environment, files and examples |

The service runs as a transient `DynamicUser` with a locked-down sandbox. It keeps no local state. Upgrades restart it.

The man page's options are generated from `--help` with help2man. The configuration half is hand-written in `debian/agent-memory-mcp.h2m`. `scripts/check-manpage.sh` fails the build if a section goes missing or an option isn't mentioned.

### Deploying

Packages go out through the local Debian repo on fuzzball, like the rest of the fleet:

1. Bump the version (see [Versioning](#versioning)). apt won't see a new package otherwise.
2. Push, wait for CI to pass, and download the `agent-memory-mcp-deb-<sha>` artifact.
3. Copy `agent-memory-mcp_<version>_amd64.deb` into the amd64 suite under `/mnt/creatures/debs` on fuzzball, then run `/mnt/creatures/debs/update_repo.sh` to regenerate `Packages.gz`.
4. On the host: `sudo apt update && sudo apt upgrade`.

On a first install, set the allowed hosts and restart:

```bash
sudoedit /etc/default/agent-memory-mcp   # set ALLOWED_HOSTS to the tailnet name(s)
sudo systemctl restart agent-memory-mcp
journalctl -u agent-memory-mcp -f
```

### Versioning

`debian/changelog` is the source of truth for the version, because its version is what apt deploys. Everything else must match it:

| File | Field |
|------|-------|
| `debian/changelog` | top entry, e.g. `agent-memory-mcp (0.2.0)` |
| `Cargo.toml` | `version` (this is also what `agent-memory-mcp --version` reports) |
| `plugin/.claude-plugin/plugin.json` | `version` |
| `plugin/.codex-plugin/plugin.json` | `version` |

`scripts/check-versions.sh` compares them. `debian/rules` runs it before building, and CI runs it as an early step, so a package with mismatched versions can't be built. To release:

```bash
dch -v 0.3.0 "What changed."
```

Then set the same version in the other three files, run `cargo build` to refresh `Cargo.lock`, and check:

```bash
scripts/check-versions.sh
```

## CI

`.github/workflows/ci.yml` runs in a `debian:trixie` container on every push to `main` and on pull requests. It checks formatting, runs clippy, runs the full test suite against a `mongo:9` service container, builds the package with `dpkg-buildpackage`, lints it with lintian, and uploads the `.deb` as a job artifact.

## Troubleshooting

**Claude Code says the server needs authentication or sign-in fails.** The server has no authentication, so this almost always means the request was rejected by the Host allow-list. Claude Code reads an HTTP 403 as "auth required", tries OAuth discovery, gets 404s, and reports a sign-in error. To confirm:

```bash
curl -i -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
  http://<host>:8787/mcp -d '{"jsonrpc":"2.0","id":1,"method":"ping"}'
```

If the response is `403 Forbidden: Host header is not allowed`, add exactly the name or IP from the URL you configured in the client to `ALLOWED_HOSTS` in `/etc/default/agent-memory-mcp`, restart the service, then reconnect from `/mcp` in Claude Code. At startup the server logs the hosts it accepts, and warns when it's listening on the network but only loopback names are allowed.

## Connecting agents

### Claude Code: the plugin (recommended)

The `agent-memory` plugin in [`plugin/`](plugin/) connects Claude Code to the server and makes it the agent's primary memory:

- **MCP connection.** The plugin bundles the server config. The URL is a plugin option and defaults to `http://10.19.63.10:8787/mcp`.
- **SessionStart hook.** Gives the agent the project key for the current repo (the bare `origin` repo name, lowercased) and checks the server's `/healthz`. It tells the agent to read the project's state and search for prior art before starting work, and to bootstrap memory when a project has none yet.
- **Skills:**
  - `agent-memory:guide`: conventions for where memories go, the single `project-state` entry per project, and titles, tags and dedup.
  - `agent-memory:bootstrap`: seeds memory for a project that predates agent-memory, from its docs, build setup, git history and the agent's local memory.
  - `agent-memory:sync`: updates the project's state and saves new techniques at the end of a piece of work.

Install it from this repo's marketplace. There are two steps: adding the marketplace only registers the catalog and doesn't install anything, so the plugin won't appear among your installed plugins until you run the install. From a shell:

```bash
claude plugin marketplace add opsnlops/agent-memory-mcp
```

```bash
claude plugin install agent-memory@agent-memory-mcp --config server_url=http://10.19.63.10:8787/mcp
```

`--config` sets the server URL during install. Set it explicitly: the install can otherwise report the option as unset even though it has a default. Use a different URL here to point at another server, for example over Tailscale.

Then restart Claude Code. A session that was open when you ran these commands won't see the plugin until it restarts or you run `/reload-plugins`. To check it worked, `claude plugin list` should show `agent-memory@agent-memory-mcp` as enabled, and `claude mcp list` should show `plugin:agent-memory:agent-memory` as connected.

You can also install from inside a session with `/plugin install agent-memory --marketplace opsnlops/agent-memory-mcp`. Then set the URL with `/plugin configure agent-memory` (**agent-memory server URL**) and restart.

To change the URL later from a shell:

```bash
echo '{"server_url":"http://10.19.63.10:8787/mcp"}' | claude plugin configure agent-memory@agent-memory-mcp --values-stdin
```

If you previously added the server by hand with `claude mcp add`, Claude Code connects to only one of the two, because they share a URL. You can remove the manual entry with `claude mcp remove agent-memory -s user`. Keep it if you want to change the URL there instead of in the plugin.

To work on the plugin, load it from the checkout instead of the installed copy, and validate it:

```bash
claude --plugin-dir ./plugin
```

```bash
claude plugin validate ./plugin && claude plugin validate .
```

### Codex: the same plugin

The `plugin/` directory is also a Codex plugin, with the same skills and the same SessionStart hook script. Install it from this repo's marketplace. As with Claude Code, adding the marketplace doesn't install anything:

```bash
codex plugin marketplace add opsnlops/agent-memory-mcp
```

```bash
codex plugin add agent-memory@agent-memory-mcp
```

Start a new Codex session, then run `/hooks` and **trust the agent-memory SessionStart hook**. Codex won't run plugin hooks until you approve them, and asks again whenever the hook script changes in a plugin update. Without the hook, the skills and MCP tools still work, but Codex isn't told the project key or reminded to check memory at the start of each session.

Codex has no plugin options, so the server URL is fixed in [`plugin/codex-mcp.json`](plugin/codex-mcp.json) (`http://10.19.63.10:8787/mcp`). To use a different URL, change that file in a fork, or skip the plugin's server and add your own to `~/.codex/config.toml`:

```toml
[mcp_servers.agent-memory]
url = "http://my-box.tailnet-name.ts.net:8787/mcp"
```

To update after changes are pushed, run `codex plugin marketplace upgrade agent-memory-mcp` and start a new session.

### How the plugin directory serves both agents

Each agent reads only its own manifest and MCP file. Everything else is shared.

| | Claude Code | Codex |
|---|---|---|
| Manifest | `.claude-plugin/plugin.json` | `.codex-plugin/plugin.json` |
| MCP server | inline in the manifest, URL from the `server_url` option | `codex-mcp.json` (fixed URL) |
| Hooks file | `hooks/hooks.json` | `hooks/codex-hooks.json` |
| Hook script | `hooks/session-start.sh` (shared) | same |
| Skills | `skills/` (shared) | same |
| Marketplace | `.claude-plugin/marketplace.json` | `.agents/plugins/marketplace.json` |

Don't add a `.mcp.json` to `plugin/`. Codex loads it by default, and Claude Code's `${user_config.server_url}` placeholder means nothing to Codex, so its server definition breaks. The `codex-mcp.json` name avoids that collision.

### Other MCP clients

Any Streamable HTTP MCP client can use `http://<host>:8787/mcp`, as long as `<host>` is in the server's `ALLOWED_HOSTS`.

## License

MIT. See [LICENSE](LICENSE).
