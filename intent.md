# Goal

I want to be able to share memories between Claude Code sessions. I want one session to be able to write something and for it to be discoverable by other Claude Code sessions. I also use Codex, so it needs to be written as an MCP server that's usable by any agent.

Let's put it on top of MongoDB 9. We already have a MongoDB 9 server on-prem, and it's on fuzzball.local.

The entire goal is to allow agents to share memories and techniques as we discover new ways of doing things.

## Tools

Let's write this in Rust. Use the latest version of the MongoDB Rust driver.

There should be no auth, assume network level auth is all we need. I will lock it behind Tailscale for remote access.

Use whatever Rust web framework makes the most sense for an MCP.
