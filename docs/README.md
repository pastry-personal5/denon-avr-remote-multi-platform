# Documentation map

Start here for current project guidance.

| Need | Authoritative document |
| --- | --- |
| Current package boundaries, session ownership, and receiver invariants | [Architecture](../ARCHITECTURE.md) |
| Target design for agent control: MCP servers (stdio and Streamable HTTP), Control API server, policy, and approval (not yet implemented) | [Planned architecture](planned-architecture.md) |
| Sequencing and milestones for implementing the planned architecture (version 4, planned) | [Version 4 roadmap](v4/roadmap.md) |
| Dedicated macOS account for the planned HTTP MCP server: setup, checks, removal (not yet implemented) | [MCP HTTP service account](mcp-http-service-account.md) |
| Engineering policy, review expectations, and documentation maintenance | [Contributing](contributing.md) |
| Prerequisites, commands, and verification gates | [Development](development.md) |
| CLI operation | [CLI user guide](cli-user-guide.md) |
| Desktop operation and logs | [Desktop user guide](desktop-user-guide.md) |
| Protocol and receiver evidence | [Research](research/) |
| Completed plans, releases, changelogs, and validation records | [Archive](archive/README.md) |

`ARCHITECTURE.md`, `contributing.md`, and `development.md` own current
architecture, policy, and commands respectively. Research supports decisions;
it is not a replacement for current architecture or policy. Completed phase
records are historical context in the archive and do not redefine the
architecture. No phase is currently active; the version 4 roadmap is planned
and no milestone has started.
