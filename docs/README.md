# Documentation map

Start here for current project guidance.

| Need | Authoritative document |
| --- | --- |
| Current package boundaries, session ownership, and receiver invariants | [Architecture](../ARCHITECTURE.md) |
| Target design for agent control: MCP servers (stdio and Streamable HTTP), Control API server, policy, and approval (not yet implemented) | [Planned architecture](planned-architecture.md) |
| Sequencing and milestones for implementing the planned architecture (version 4, planned) | [Version 4 roadmap](v4/roadmap.md) |
| Phase 4, planned and not started: the Control API server, its `/v1` contract, and the client that implements the port over a Unix socket | [Phase 4 overview](v4/phase-4-control-api-server-overview.md) and [architecture](v4/phase-4-control-api-server-architecture.md) |
| Phase 3, implemented and awaiting the owner's live checks: policy engine, audit log, and the Agent path through the gate | [Phase 3 overview](v4/phase-3-policy-audit-gate-overview.md) and [architecture](v4/phase-3-policy-audit-gate-architecture.md); the validation record is in the [archive](archive/v4/phase-3-live-validation-record.md) |
| Phase 2, implemented and awaiting the owner's checks: GUI on the control-service port and retirement of the legacy controller | [Phase 2 overview](v4/phase-2-gui-on-port-overview.md) and [architecture](v4/phase-2-gui-on-port-architecture.md) |
| Previous phase, complete: control port and in-process service | [Phase 1 overview](v4/phase-1-control-port-overview.md) and [architecture](v4/phase-1-control-port-architecture.md) |
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
architecture. Version 4 milestone 1 is complete. Milestones 2 and 3 are
implemented and wait on the owner's visual and live checks. Milestone 4 is
planned and not started; later milestones in the version 4 roadmap have not been
planned in detail.
