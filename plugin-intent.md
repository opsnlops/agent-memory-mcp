# Goal

A Claude Code PLugin that tells the agent to prefer the agent-memory-mcp for memory storage.

## Discussion

* The agent should know to look at the MCP first before starting something new to see if there is prior art for techniques and context before beginning.
* The agent should keep the memory MCP up to date with the current state of a project.
* If the agent comes across a project that pre-dates the memory MCP, it should dump the context it knows into the memory MCP.
