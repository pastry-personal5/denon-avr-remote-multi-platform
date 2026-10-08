# Planned architecture

This document describes the target design for agent-driven receiver control.
[ARCHITECTURE.md](../ARCHITECTURE.md) remains the authoritative description of
the implemented system and its receiver-correctness invariants; this document
does not restate or override them. Text here moves into `ARCHITECTURE.md` when
the corresponding design is implemented and is then removed from this file.
Sequencing is decided as work proceeds and is deliberately not recorded here.

## Purpose and scope

The receiver is controlled today by a desktop GUI and a short-lived CLI. Each
opens its own receiver connection and neither applies an authorization policy.
This design adds entry points for AI agents (such as OpenClaw, Dots, Muse, and
Hermes) through MCP and makes them safe: a state-changing request from an agent
is classified by a Policy Engine, and a request classified as dangerous waits
for the user's approval, obtained through an external approval service.

The MCP server is delivered as two builds so that the exposure matches where
the agent runs. A **stdio** build serves an agent on the Control API server's
own machine and has no network listener. A **Streamable HTTP** build serves
agent hosts elsewhere on the LAN. Both expose the same tools and pass through
the same gate. Nothing in the design is specific to one agent, and several
agents can connect at once.

Goals:

- An agent can read receiver state and request control operations over MCP,
  over stdio or over Streamable HTTP.
- Danger is classified deterministically, with knowledge of current receiver
  state, in one place that none of this repository's binaries can bypass.
- A dangerous operation runs only after the user approves that exact operation.
- Each receiver connection has one owner. The GUI, CLI, and MCP servers are
  clients of one control surface.
- Each MCP build carries only the transport code it uses. The stdio build has
  no network listener, and that is a property of its dependency graph.
- Agents on the HTTP build can authenticate with standards-based OAuth as well
  as with static tokens, using short-lived credentials that the user can revoke.

Non-goals:

- Building or operating the External Approval Service, which is a separate
  project. Only the consuming side of its contract is defined here.
- Defending against a hostile process that can reach the receiver directly. The
  receiver's LAN protocols are unauthenticated; see
  [Security model](#security-model).
- Constraining an agent that can run commands or read files under the same
  operating-system account as the user's operator tools. Software here cannot;
  see [Choosing a build](#choosing-a-build).
- Remote (off-LAN) access, multi-user accounts, and HEOS control.
- Windows and Linux in the first release, which supports macOS only. The
  Windows named-pipe transport and its access control remain the intended later
  direction, and nothing here commits to either system.
- Quick Select recall and EQ status. Neither is a receiver intent or part of the
  control surface. Each would return as a classified intent with its own tool,
  as [Policy Engine](#policy-engine) requires of every intent.
- Operating-system services. The design uses no launchd, Keychain, or similar
  facility: processes are started from a terminal, by the GUI, or by an agent
  host, and credentials live in owner-only files and the agent host's
  configuration. Creating the separate operating-system account that some
  deployments require is left to the user.
- Hardening the AI agents themselves.

Terminology. Two different servers appear in this design and are always named
in full: the **Control API server** is part of this system and owns the
receiver; the **External Approval Service** is an independent external entity
outside this system. The **MCP server** is two builds of one tool surface:
**mcp-stdio** and **mcp-http**. The Control API server listens on two local
endpoints: the **Agent endpoint** and the **Operator endpoint**. The optional
**authorization server** is a separate service that issues OAuth access tokens
for the HTTP build. An **operation** is a single state-changing request, such
as setting the volume. An **agent host** is the machine on which an AI agent
runs.

## System context

```text
   ┌────────────┐
   │ AI agents  │  other hosts on the LAN, or this machine in a separate OS account
   └─────┬──────┘
         │ MCP over Streamable HTTP with TLS, or over stdio
   ┌─────▼──────┐       ┌───────────┐      ┌───────┐
   │ MCP server │       │  GUI app  │      │  CLI  │
   │ two builds │       └─────┬─────┘      └───┬───┘
   └─────┬──────┘             │                │
         │ Agent endpoint     └───────┬────────┘
         │                            │ Operator endpoint
 ┌───────▼────────────────────────────▼──────────────┐
 │ Control API server                 trust boundary │       ┌───────────────────┐
 │                                                   │       │ External Approval │
 │   Operation Gate ─┬─ Policy Engine                │       │ Service           │
 │                   └─ Approval Broker ─────────────┼──────►│ (out of scope)    │
 │   Audit log                                       │       └─────────┬─────────┘
 │   Receiver core (single session owner)            │                 │ asks
 └──────────────────┬────────────────────────────────┘             ┌───▼───┐
                    │ AVR TCP / HTTP                               │ User  │
              ┌─────▼─────┐                                        └───────┘
              │ Receiver  │
              └───────────┘
```

Every client reaches the receiver only through the Control API server. Both MCP
builds, the GUI, and the CLI contain no receiver adapter, and the dependency
rules in [Workspace and dependencies](#workspace-and-dependencies) make that a
checked property rather than a convention. When OAuth is enabled, an
authorization server (not shown) runs beside `mcp-http`. It issues tokens, has
no receiver path, and can mint only Agent credentials.

## Design principles

1. **One trust boundary.** The Control API server owns the receiver session,
   policy evaluation, approval, and audit. The MCP servers are untrusted
   adapters; the GUI and CLI are trusted to speak for the user.
2. **Enforce where the session lives.** The gate sits in front of the only call
   that dispatches writes. Client-side controls such as MCP tool annotations,
   host approval prompts, and tool allow-lists are welcome extra layers, but
   none is relied on: a host can run in a mode that never prompts, and a
   host-side grant can be keyed by tool name rather than by arguments.
3. **Fail closed.** Missing policy, an unreachable approval service, an
   unclassified intent, or an unknown receiver state never turns into an
   allowed operation.
4. **Approval is bound to what the user saw.** An approval covers one exact
   operation, receiver, observed baseline, and expiry, and is used once.
5. **Honest outcomes.** Results distinguish not dispatched, dispatched but
   unconfirmed, and confirmed by receiver evidence. Nothing implies success from
   an acknowledgement.
6. **The agent cannot approve itself or edit policy.** No agent-facing surface
   can grant approval, change policy, or read credentials, and the Agent
   endpoint serves no Operator resource at all.
7. **Policy is pure and data-driven.** Classification is a deterministic
   function of typed inputs and a declarative configuration.
8. **Minimal exposure per build.** Each MCP build contains only the transport it
   uses. A transport that is not needed is not present to be attacked.

## Components

| Component | Package | Responsibility | Relative to current code |
| --- | --- | --- | --- |
| Receiver core | `domain`, `protocol`, `application`, `infrastructure` | Receiver model, wire protocols, serialized session, evidence-based state | Existing; contract additions, see below |
| Operation Gate | `application` | The only path from a request to a dispatching session call | New |
| Policy Engine | `policy` | Pure classification of an operation as allow, require approval, or deny | New |
| Approval Broker | `application` (port), `infrastructure` (adapter) | Creates, tracks, verifies, and expires approval requests | New |
| Audit log | `application` (port), `infrastructure` (adapter) | Append-only record of every operation and decision | New |
| Control API server | `apps/api-server` | Hosts the above, authenticates callers, serves the Agent and Operator endpoints | New |
| Control API contract | `api-contract` | Wire schema shared by server and client | New |
| Control API client | `api-client` | Implements the application control-service port over the API, over local transports only | New |
| MCP tool surface | `mcp-tools` | Tool definitions, argument validation, and result mapping; no transport | New |
| MCP stdio server | `apps/mcp-stdio` | The tool surface over stdio, for an agent on the server's machine | New |
| MCP HTTP server | `apps/mcp-http` | The tool surface over Streamable HTTP, for agent hosts on the LAN | New |
| Authorization server | `apps/auth-server` | Optional OAuth 2.1 server for the HTTP build: issues short-lived Agent access tokens and exchanges them for tokens the Agent endpoint accepts | New |
| GUI app | `gui-lib`, `apps/desktop` | Presentation, supported on macOS in the first release; an Operator client | Changed |
| CLI | `apps/cli` | Operator commands and token management; an Operator client | Changed |
| Diagnostics | `apps/diagnostics` | Explicit read-only evidence probes | Existing; unchanged |
| External Approval Service | none | Asks the user and returns a signed decision | External |

### Receiver core and the session contract

The Operation Gate and Control API server are written against the state-first
`CanonicalReceiverSession` contract, defined in
`crates/application/src/session_v3.rs`. Policy needs complete receiver state
with per-field validity, and callers need `OperationOutcome` with its
`DispatchCertainty`; both are first-class there. The GUI's present path (the
legacy controller over a compatibility adapter) was replaced in milestone 2 by
projecting the same state and operation results from the in-process control
service; milestone 5 retargets that to the Control API. No third session contract
is introduced. `ARCHITECTURE.md`, `AGENTS.md`, and the single-definition rule in
`tools/check-boundaries.sh` name `CanonicalReceiverSession` and its definition
site, and the legacy contracts and the controller they served are retired. See
[Desktop GUI](../ARCHITECTURE.md#desktop-gui).

The precondition, the typed rejection cause, and the inspection reads are
implemented. See [Control service](../ARCHITECTURE.md#control-service). The
sections below keep their headings so links resolve, and are removed with the
release milestone.

### Receiver identity

Implemented. See [Control service](../ARCHITECTURE.md#control-service).

### Receiver connection

Implemented. See [Control service](../ARCHITECTURE.md#control-service).

### Control service port

Implemented in process. See [Control service](../ARCHITECTURE.md#control-service).
What remains is `api-client`, which implements the same port over the Control
API, and the token, approval, audit, and policy views on `OperatorAdmin`.

### Process model

| Process | Started by | Lifetime |
| --- | --- | --- |
| Control API server, standalone | A command in a terminal | Until stopped; serves the MCP servers, CLI, and GUI |
| Control API server, GUI-owned | The GUI app, as a child process, when no server is running | Ends when the GUI closes; the child also exits when the pipe from its parent closes, so a GUI crash ends it too |
| MCP stdio server | The agent host, as a child process, from its MCP configuration | One agent session; exits when its standard input closes; never starts or stops the Control API server |
| MCP HTTP server | A service the user runs on the Control API server's machine | Until stopped; keeps no state it relies on; serves agent hosts over the LAN; never starts or stops the Control API server |
| Authorization server | A service the user runs under its own account; needed only when OAuth is enabled | Until stopped; holds the token signing key; never starts or stops any other process |
| GUI app | A command in a terminal or the installed package | Interactive; attaches to a running server or spawns its own |
| CLI | A command in a terminal | Short-lived; fails with a clear message when no server is running |

- There is one Control API server per user. It binds both local endpoints
  exclusively, and a second server refuses to start. A GUI that finds a running
  server attaches to it and never stops a server it did not start.
- Agent access exists only while a server runs. When a GUI-owned server ends,
  pending operations expire under the lifecycle rules below, and both MCP
  servers report the receiver service as unavailable instead of guessing.
- The GUI starts the server by launching its executable, not by linking it, so
  the GUI keeps no code path to the receiver.
- Neither MCP build needs the Control API server at start. Each connects when a
  tool is called, so an agent host's handshake succeeds while the server is down
  and the tool call reports the service as unavailable.

## Operation lifecycle

Every state-changing request, whatever its origin, becomes an operation owned
by the gate. An Operator request is evaluated trivially as `Allow`, tagged as
Operator, and follows the same states from there.

| From | To | When |
| --- | --- | --- |
| `submitted` | `evaluated` | Always; the Policy Engine runs |
| `evaluated` | `denied` | Decision is `Deny`; never dispatched |
| `evaluated` | `allowed` | Decision is `Allow` |
| `evaluated` | `awaiting_approval` | Decision is `RequireApproval` and the approval channel is available; a ticket is created |
| `evaluated` | `approval_unavailable` | Decision is `RequireApproval` and the approval channel is unconfigured or unreachable; never dispatched |
| `awaiting_approval` | `approved` | A verified `approved` decision arrives |
| `awaiting_approval` | `approval_rejected` | A verified `rejected` decision arrives; never dispatched |
| `awaiting_approval` | `expired` | Ticket expiry or server restart; never dispatched |
| `awaiting_approval` | `cancelled` | The owning principal cancels; never dispatched |
| `approved` | `denied` | Re-evaluation at dispatch is `Deny` |
| `approved` | `awaiting_approval` | Re-evaluation requires approval again or the baseline changed; the old ticket is void and a new one is created |
| `allowed`, `approved` | `in_session` | Re-evaluation permits it; the session's `operate` is called and the approval ticket is consumed |
| `in_session` | a terminal status | The session's `OperationOutcome`, mapped as below |

The session's outcome is reported verbatim and is not reinterpreted. The first
column of the mapping is the session's; the other three are what clients see:

| Session outcome | Status | `dispatch` | `confirmed` |
| --- | --- | --- | --- |
| `ObservedRequestedValue` | `completed` | As reported: `complete_write`, or `possibly_dispatched` or `unknown` after an ambiguous write | true |
| `AlreadyObserved` | `already_in_state` | `not_dispatched` | true |
| `RejectedBeforeDispatch` | `rejected`, with the session's reason | `not_dispatched` | false |
| `Cancelled` | `cancelled` | `not_dispatched` | false |
| `SupersededBeforeDispatch` | `superseded` | `not_dispatched` | false |
| `Indeterminate` | `indeterminate` | As reported | false |

Every status decided before the session (`denied`, `awaiting_approval`,
`approval_rejected`, `expired`, `approval_unavailable`, `cancelled`) reports
`dispatch` as `not_dispatched` and `confirmed` as false. `dispatch` is the
session's `DispatchCertainty` and nothing more: `complete_write` means the local
write completed, not that the receiver acknowledged it. `confirmed` is true only
when receiver evidence shows the requested value.

```text
AI agent    MCP server    Control API server                Approval Service   User
   │ set_volume  │               │                                 │              │
   │────────────►│ submit        │                                 │              │
   │             │──────────────►│ policy → require approval       │              │
   │             │               │ submit(ApprovalRequest)         │              │
   │             │               │────────────────────────────────►│ ask          │
   │             │◄──────────────│ awaiting_approval + operation id│─────────────►│
   │◄────────────│               │                                 │              │
   │ get_operation               │                                 │   decision   │
   │────────────►│──────────────►│                                 │◄─────────────│
   │             │               │◄────────────────────────────────│ signed decision
   │             │               │ verify · re-evaluate · operate once · confirm  │
   │◄────────────│◄──────────────│ completed | indeterminate | ...                │
```

Invariants:

- **Server-allocated identity and ownership.** The server assigns operation
  ids. Clients supply at most an idempotency key. The CLI's present practice of
  a constant id goes away. An operation is owned by its principal, an Operator
  or one labelled Agent token, and not by an MCP session or a transport, so an
  agent whose stdio process restarted or whose HTTP connection dropped can still
  read and cancel its operations. An identical in-flight operation from the same
  principal returns the existing operation instead of creating another, so a
  client that retries after a timeout cannot dispatch twice. Every intent is a
  target condition that the session tests against observed state (the sound-mode
  recall intents target a category rather than one mode), and none steps relative
  to the current value, so a retry after completion is observed as already in
  state rather than applied again.
- **Waiting is off the session.** An operation awaiting approval never occupies
  the serialized session queue. The session sees an operation only once it is
  cleared, and the existing at-most-once dispatch rule then applies unchanged.
  The broker never re-submits.
- **Re-evaluation at dispatch.** Receiver state can change while the user
  decides. The gate evaluates policy again just before dispatch. An approval
  stays valid only if the new evaluation is no more severe and the baseline the
  approver saw is still current; otherwise the operation needs a new approval or
  is denied. A reconnect or receiver change invalidates the approval, matching
  the existing rule that such events invalidate authority from the old
  connection. The precondition on the session request closes the remaining
  window between this evaluation and the write. Evaluation needs the receiver's
  state, so it runs under a lease on the session after the receiver is connected
  and synchronized: an Agent submit returns `submitted` at once and the gate's task
  evaluates. While no approval can intervene, nothing waits between evaluation and
  dispatch, so the one evaluation is the one at dispatch. A precondition mismatch
  ends the operation as `rejected`, and the gate never retries it.
- **An approval is spent when the session is called.** If the session then
  returns `rejected`, `superseded`, or `cancelled`, nothing was dispatched but
  the ticket is consumed; a new attempt needs a new approval. A newer operation
  superseding an approved one passes its own gate, so supersession cannot
  dispatch anything unapproved.
- **Bounded re-approval.** An operation may request approval again once because
  its baseline changed. A second change ends it as `denied` with reason
  `baseline_unstable`, so a moving baseline cannot keep prompting the user.
- **Bounded pending work.** Pending approvals per client, request rate, and
  approval lifetime are capped. Pending operations are not durable: a server
  restart expires them and a decision for an unknown ticket is rejected and
  audited.
- **Cancellation.** A client may cancel its own operations only before
  `in_session`. Cancellation also withdraws the approval ticket. A cancel that
  arrives later is answered as too late, with the operation's actual status. The
  gate never abandons a call into the session, because the session infers
  cancellation from a dropped call and its outcome would be lost; the session's
  `Cancelled` outcome therefore appears only when the call is dropped, such as
  at server shutdown.

## Policy Engine

The Policy Engine is a pure library that depends only on `domain`. It performs
no I/O, reads no clock, and holds no state. The gate keeps the ledger of recent
volume changes on each receiver and passes it in: the agents' operations
(reserved before dispatch, dispatched, or possibly dispatched) and the
Operator's volume writes, each with the level the receiver showed and the level
that was asked for. The gate rebuilds the ledger from the audit log at start, so
a server restart does not reset the budget.

```text
evaluate(PolicyInput, PolicyConfig) -> Decision

PolicyInput = agent label, receiver id, requested intent,
              current receiver state (with per-field validity),
              the receiver's recent volume changes,
              wall-clock time (injected)
Decision    = Allow            { baseline }
            | RequireApproval  { reasons, matched rules, baseline }
            | Deny             { reasons, matched rules }
```

`Deny` is a limit that approval cannot override. `RequireApproval` is a limit
that the user may waive for one operation. Where several rules match, the most
restrictive decision wins, in the order `Deny`, `RequireApproval`, `Allow`.

**Baseline.** The baseline is the receiver epoch plus the value and validity of
every field read by a state-dependent condition of any rule that *applies* to the
request, whether or not that rule matched, and it comes with `Allow` as well as
`RequireApproval`. A rule applies when its scope, intent, and value all match the
request, so its outcome depends only on the state it reads. An unmute is allowed
because the loud-baseline rule did not match; the baseline must still carry the
volume, or the write could follow a change in volume the decision never saw. A
field that no applicable rule reads is not in the baseline, so an unrelated
change does not void an approval or an allowed write. The gate turns the baseline
into the session request's precondition.

| Dimension | Examples of what a rule can express |
| --- | --- |
| Intent | System power, main zone power, Zone 2 power, source, volume, mute, sound mode |
| Magnitude against baseline | A volume increase over a step limit; a target over a ceiling |
| State validity | A stale, unknown, or unavailable baseline for a state-dependent rule counts as dangerous |
| Interaction | Power-on or unmute while the volume is above a ceiling, or unknown |
| Cumulative effect | A budget over a time window, pooled across all agents on a receiver, so neither a series of small steps nor several agents splitting a change can evade a per-step limit |
| Context | Agent label, receiver |

The first release has no time-of-day dimension. A rule needing one waits for a
later release, and the policy schema rejects the key rather than ignoring it.

An intent is **classified for an agent** when at least one rule that applies to
that agent, of any decision, names the intent: its kind, and its value when the
rule gives one, so a rule naming `system_power` with value `standby` classifies
standby and leaves power-on unclassified. A rule that names no intent
restricts but never classifies. A classified intent that matches no restricting
rule is `Allow`; an unclassified intent is treated as dangerous
(`RequireApproval`). Classification is per agent: a rule narrowed to one label
classifies the intent for that label only, so it cannot turn another agent's
unclassified intent into `Allow`. The domain model may gain intents; each gets a
tool and a classification together, and the policy crate maps every
`ReceiverIntent` variant to an intent kind with an exhaustive match, so a new
variant does not compile until it is mapped.

An `allow` rule exists to classify an intent and never relaxes a restricting
rule, so it may carry a scope, an intent, and a value and nothing else; the
loader rejects an `allow` rule with any other condition. A `when` that names only
`agents` or `receivers`, and no intent, matches every intent. That is how a rule
says "deny every write for this label".

Volume values are the relative decibel scale the CLI already uses, from -79.5
to +18.0 in 0.5 dB steps, plus the receiver's `Minimum`, which the rules treat as
-80.0 dB. A target of `Minimum` is therefore always a decrease, and a rise from
`Minimum` is measured conservatively. Limits in policy must lie on the 0.5 dB
grid.

The cumulative budget is the rise of the requested level above the lowest level
in the window, and it applies only to an increase: a target at or below the
observed level is never over the budget, however deep the floor, so a decrease
from a usable level is not held back by the step or the budget. That floor is the
lowest of the receiver's currently observed level and, for every ledger entry in
the window, the level the receiver showed before the change and the level that was
asked for. Entries are agent operations
that are reserved, dispatched, or possibly dispatched (approved ones included),
and Operator volume writes. A decrease lowers the floor the next rise is measured
from, so alternating up and down steps cannot reset the budget. The window is
wall-clock time, because it must survive a restart. An entry dated later than now
(the clock stepped back) still counts, so a backward step cannot shrink the
budget; a forward step can expire entries early, which is accepted. The budget is
pooled across agents. The gate reads the ledger, evaluates, and reserves the
operation's entry in one step under a lock, so two agents cannot each pass a
budget that together they exceed.

The shipped default policy classifies agent operations as follows:

| Operation | Default decision |
| --- | --- |
| Volume target above the hard limit | `Deny` |
| Volume target above the ceiling | `RequireApproval` |
| Volume increase over the step limit from the observed level, or over the budget within the time window | `RequireApproval` |
| Any other volume change, including every decrease, when the observed volume is usable | `Allow` |
| Any volume change when the observed volume is unknown or stale | `RequireApproval` |
| Main zone power on, or unmute, when the observed volume is above the ceiling or unknown | `RequireApproval` |
| Main zone power on, or unmute, when the observed volume is known and at or below the ceiling | `Allow` |
| System power on or off | `RequireApproval` |
| Zone 2 power on or off | `RequireApproval` |
| Main zone power off, mute on, source change, sound mode change | `Allow` |
| Any unclassified intent | `RequireApproval` |

The numeric limits (hard limit, ceiling, step, budget, window) have no shipped
values. If a limit is not configured, every operation its rule matches requires
approval, so until limits are configured an agent needs approval for every
volume change and every power-on. System power on needs approval whatever the
volume. If the receiver reports no usable volume in standby, a main zone
power-on from standby needs approval under the unknown-baseline rule. The step
and budget rules cannot be checked without a usable observed volume, so a volume
change then needs approval too, a decrease included, because it cannot be shown
to be one.
Approval lifetime is also configured in policy and defaults to a few minutes.

Policy is a YAML document loaded by an infrastructure adapter into a typed
`PolicyConfig`. The following is illustrative only; the numbers are examples,
not defaults. Rules under `agent` apply to every agent, and a rule can be
narrowed to some agents by an `agents` list of labels, or to some receivers by a
`receivers` list of ids, in its `when`. The loader rejects `unclassified: allow`,
an `or_unknown` that is not `true` (an unknown or stale volume always matches),
limits off the 0.5 dB grid, an `allow` rule with a condition beyond scope, intent,
and value, and unknown keys, including any time-of-day key:

```yaml
unclassified: require_approval
agent:
  approval_lifetime_minutes: 5
  rules:
    - id: volume-hard-limit
      when: { intent: volume, target_above_db: -10.0 }
      then: deny
    - id: volume-ceiling
      when: { intent: volume, target_above_db: -30.0 }
      then: require_approval
    - id: volume-step
      when: { intent: volume, increase_over_baseline_db: 6.0 }
      then: require_approval
    - id: volume-budget
      when: { intent: volume, rise_over_window_db: 10.0, window_minutes: 10 }
      then: require_approval
    - id: loud-or-unknown-baseline
      when:
        any:
          - { intent: main_zone_power, value: on }
          - { intent: mute, value: off }
        baseline_volume: { above_db: -30.0, or_unknown: true }
      then: require_approval
    - id: system-power
      when: { intent: system_power }
      then: require_approval
    - id: zone2-power
      when: { intent: zone2_power }
      then: require_approval
    - id: low-risk
      when:
        any:
          - { intent: main_zone_power, value: off }
          - { intent: mute, value: on }
          - { intent: source }
          - { intent: sound_mode }
      then: allow
    # Narrowed to one label and naming no intent: deny every write for it.
    - id: claude-code-read-only
      when: { agents: [claude-code] }
      then: deny
```

Policy is authored by editing the file (`policy.yaml`, beside the receiver
configuration); no API route writes it. A change takes effect on server start or
an explicit Operator reload, which validates the file and replaces the active
policy as a whole. A load that fails, at start or on reload, leaves no active
policy: agent writes are disabled while reads continue, until a later load
succeeds. The digest is the SHA-256 of the file bytes that were read. It is shown
with the effective policy and recorded in the audit log each time a policy is
loaded. The Policy Engine cannot know how loud a source is or what a given level
means in a given room; its limits complement any receiver-side limits and do not
replace them.

## Audit

The audit log is a port in `application` with an append-only JSON Lines adapter.
Each record is one event and carries a schema version, the server run (its start
time) it belongs to, the operation id, the principal, the receiver, and the
wall-clock time. Operation ids restart at 1 with every run, so a record is matched
to its operation by run and id. Agent-supplied text is bounded and escaped.

| Event | Written | Carries |
| --- | --- | --- |
| `decided` | When the policy decision is reached | The intent, the decision, reasons, matched rule ids, the baseline, and the policy digest |
| `dispatching` | Before the session's `operate` is called | The requested level and the level the receiver showed, for a volume change, and the precondition's fields |
| `finished` | When the operation ends | Status, `dispatch`, `confirmed`, and the reason |
| `policy_loaded`, `policy_load_failed` | Each load attempt | The digest, or the error |

Approval events join these with the broker. The write order is part of the
contract. The `dispatching` record is written and synced to disk before the
session is called, and a failed append rejects an Agent operation and lets an
Operator one continue with a warning. After a crash, the ledger is rebuilt from the
`dispatching` records: each counts as possibly dispatched unless a `finished`
record for the same operation says `not_dispatched`, so one with no `finished`
record counts. A `finished` record with no `dispatching` record is ignored. A
reader skips records and fields it does not know, and a truncated last line.

## Approval

The Approval Broker is the consumer of an `ApprovalChannel` port. The
infrastructure adapter speaks to the External Approval Service; this repository
defines only the consuming side of the contract.

Approval request, sent by the Control API server:

| Field | Meaning |
| --- | --- |
| `ticket_id` | Server-generated, unique |
| `operation_digest` | Hash over the canonical form of receiver, intent, baseline, principal, and expiry |
| `summary` | Human-readable description generated by the server from the typed intent and baseline |
| `risk` | Rule identifiers and reasons from the Policy Engine |
| `requester` | The agent's label from its token registration, never self-reported, plus any agent-supplied justification, marked untrusted, plain text, and length-limited |
| `expires_at` | Absolute expiry |
| `nonce` | Single-use value echoed in the decision |

Approval decision, required from the External Approval Service:

| Field | Requirement |
| --- | --- |
| `ticket_id`, `operation_digest`, `nonce` | Must equal the request's values |
| `outcome` | `approved` or `rejected` |
| `decided_at` | Must precede `expires_at` |
| `approver` | Opaque identifier, recorded in the audit log |
| `key_id` | Names which of the service's public keys signed the decision |
| `signature` | Public-key signature over the canonical encoding of every other field, verified with the named key |

The Control API server accepts a decision only if all of these hold, its own
clock at receipt is before `expires_at`, and the ticket has not been used. The
server's clock, not the service's, decides expiry. An unverifiable, expired,
mismatched, or replayed decision is ignored and audited.

- Decision signatures are asymmetric. The server holds only the External
  Approval Service's public keys, so an agent that can read the server's files
  cannot forge a decision. A shared-secret scheme would not have this property
  and is not acceptable.
- The approval text is always generated by the server from the typed intent.
  Free text from an agent is shown only as a labelled, untrusted justification,
  because an agent that has read hostile content could otherwise write the
  approval prompt.
- If the External Approval Service is unconfigured or unreachable, operations
  that require approval end as `approval_unavailable`. They are never approved
  by default.
- The Control API server only makes outbound connections to the External
  Approval Service. It holds a stream open (server-sent events or a WebSocket)
  and receives decisions over it, and it exposes no callback route, so approvals
  need no inbound port. The authority of a decision comes from its signature,
  not from the channel it arrived on.
- The stream must be resumable from a cursor. A dropped connection does not lose
  a decision: the server reconnects with backoff and resumes from the last event
  it processed. A pending ticket survives a dropped stream and still expires at
  `expires_at`; while the stream cannot be established, new operations that
  require approval end as `approval_unavailable`.
- The GUI shows pending and decided approvals and can cancel a pending
  operation. It cannot approve one. A single approval channel keeps the trust
  model small.

## Control API

The Control API is HTTP with JSON bodies and server-sent events, described by a
schema in `api-contract`. Paths carry a major version (`/v1`). The schema is the
source of truth, and clients ignore fields they do not know so the contract can
grow compatibly.

| Resource | Purpose | Served to |
| --- | --- | --- |
| `GET /v1/receivers` | Configured receivers. The Agent view carries identity, model, and capabilities, never network addresses | Both endpoints |
| `POST /v1/receivers/discover` | Discover receivers on the LAN | Operator endpoint |
| `GET /v1/config`, `PUT /v1/config` | Receiver configuration and per-receiver preferences | Operator endpoint |
| `GET /v1/receivers/{id}/state` | Complete receiver state with per-field validity | Both endpoints |
| `GET /v1/receivers/{id}/events` | Event stream of coalesced state snapshots and operation events. An Agent sees only its own principal's operation events | Both endpoints |
| `POST /v1/receivers/{id}/operations` | Submit an operation; `dry_run` returns the policy decision for the caller's own principal without dispatching, and the Operator may name an agent label to see that agent's decision. A dry run creates no operation and writes no audit record | Both endpoints |
| `GET /v1/operations/{id}`, `POST /v1/operations/{id}/cancel` | Status and cancellation | Both; the owner only, and the Operator sees all |
| `GET /v1/approvals` | Pending and decided approvals | Operator endpoint |
| `GET /v1/audit` | Query the audit log | Operator endpoint |
| `GET /v1/policy`, `POST /v1/policy/reload` | Effective policy and its digest; validate the file and make it active | Operator endpoint |
| `POST /v1/tokens`, `GET /v1/tokens`, `DELETE /v1/tokens/{id}` | Issue a static Agent token (the value is shown once), register an OAuth client under an agent label, list labels, revoke either | Operator endpoint |
| `GET /v1/health` | Readiness of session, policy, approval channel, audit | Both endpoints |

State snapshots are coalesced so that a slow client sees the newest complete
state, the same semantics as the session's state subscription. Operation events
are not coalesced; a client that missed one reads `GET /v1/operations/{id}`.

Transport and identity:

- The Control API is reachable only locally, over a Unix domain socket. The
  first release supports macOS only, so it implements no other transport; a
  named pipe is the intended later direction for Windows. The Control API
  server has no network listener. Only `mcp-http` and, when OAuth is enabled,
  the authorization server listen on the network, and `mcp-stdio` listens on
  nothing.
- There are two endpoints. The **Operator endpoint** lives in a per-user
  location with owner-only access (file mode or ACL), accepts connections only
  from the owner's account where the OS reports peer credentials, and serves
  every resource to the Operator token. The **Agent endpoint** serves only the
  resources marked Both, only to Agent tokens, and nothing else exists on it:
  an Operator resource is a not-found there, and an Operator token presented
  there is rejected and audited. Its location is separate and its permissions
  can admit a dedicated agent or MCP service account. If the server cannot apply
  the configured permissions to the Agent endpoint, it does not create it.
- Two principal classes exist. **Agent** is always subject to the Policy
  Engine. **Operator** requests still pass through the Operation Gate for
  server-side ids, coalescing, audit, and at-most-once dispatch, but skip policy
  classification because the human is the actor.
- Static credentials are random bearer tokens issued by the server, which keeps
  only their hashes (a fast hash is enough for high-entropy tokens; comparison
  is constant time). A token can be revoked and rotated. The server creates the
  Operator token on first start and writes it to an owner-only file that the GUI
  and CLI read. Each AI agent has its own labelled Agent token, so audit and
  policy can tell agents apart and one token can be revoked without affecting
  the others. A caller can neither choose nor change its own principal or label.
- A token is presented with every request in an `Authorization` header and never
  in a URL.
- The Agent endpoint accepts a second kind of Agent credential: an access token
  signed by the authorization server. The Control API server holds only the
  authorization server's public keys, verifies the signature, expiry, and an
  audience naming the Agent endpoint, and honors the token only while its client
  is registered under an agent label. Revoking the registration takes effect at
  once, without waiting for the token to expire. An authorization-server token
  is never accepted on the Operator endpoint. The authorization server learns
  the registered clients from a read-only export that the Control API server
  writes; it has no connection to either endpoint.
- The distinction between Operator and Agent is one of endpoints and
  credentials, not of people: any process that can read the Operator token file
  acts as Operator, which is an accepted property described under
  [Security model](#security-model).

## MCP servers

Both builds translate MCP tool calls into Control API calls on the Agent
endpoint. They hold no receiver connection, make no policy decision, and cannot
approve anything. A defective MCP server can reach only the Agent endpoint.
Whether a compromised one stays within Agent authority depends on the
operating-system account it runs under; see [Choosing a build](#choosing-a-build).

### Choosing a build

| Agent host | Build | What must hold |
| --- | --- | --- |
| Another machine or a VM on the LAN | `mcp-http` | TLS, an Agent credential (a static token, or an OAuth token when enabled), one bound address, and network rules that let the agent host reach the MCP endpoint but not the receiver |
| The Control API server's machine, as a separate operating-system account | `mcp-stdio` | The account can reach the Agent endpoint but not the owner's data directory, and an operating-system firewall rule keeps it from the receiver |
| The Control API server's machine, under the same account as the Operator tools | Either, but not a safety configuration for an agent that can run commands or read files | Anything running as the account can read the Operator token, so policy binds only an agent restricted to MCP tools |

The choice is made by where the agent runs, not by preference. The stdio build
is started by the agent host, so it runs on the agent's machine; a remote agent
host could use it only if the Control API were reachable over the network, which
this design does not allow. The HTTP build needs a listener, and a listener is
exposure that a same-machine agent does not need.

The same reasoning applies to the process that runs `mcp-http`. When it shares
the owner's account, a remote-code-execution defect in it would let an attacker
read the Operator token. Run it under a dedicated account that can reach only
the Agent endpoint and its own TLS key when that risk is not acceptable; a
macOS procedure is in [MCP HTTP service account](mcp-http-service-account.md). A
firewall rule that matches the agent's account (for example nftables on Linux or
`pf` on macOS) is the usual way to keep a same-machine agent from the receiver;
this has not been validated for this project and must be tested before it is
relied on.

### Shared tool surface (`mcp-tools`)

`mcp-tools` defines the tools and maps Control API results to tool results. It
contains no transport. It receives a control-service handle bound to one
credential: the stdio build creates one at start, and the HTTP build creates one
per request from the credential that request carries (in OAuth mode, the token
obtained by exchange), so principal and ownership follow the caller on every
request.

- **Tools only.** Resources and prompts are not used. Every capability flows
  through the same gate, and a tool-only server works with the widest range of
  agent hosts. The server sends no unsolicited messages.
- **No approval through the agent host.** MCP elicitation and host-side prompts
  are not used as the approval mechanism. The approval channel must be
  independent of the agent host the policy guards against. Hosts may add their
  own prompts; a host that auto-approves this server's tools loses no safety.
- **Annotations.** Tools carry accurate read-only and idempotency annotations.
  They are hints. Danger depends on arguments and current state, which a static
  annotation cannot express; that is the reason classification lives in the
  Policy Engine.
- **Instructions.** The server's instructions, delivered in the protocol's
  discovery or initialization exchange (the revision decides which), state the
  approval flow and the rule not to retry while an operation is pending.

| Tool | Kind | Notes |
| --- | --- | --- |
| `list_receivers` | Read | Configured receivers and their capabilities |
| `get_receiver_state` | Read | Per-field value with validity and freshness; unavailable data is reported as unavailable |
| `list_sources` | Read | Source catalog for the receiver |
| `get_operation` | Read | Status of an operation, optionally waiting for a bounded time |
| `set_power` | Write | System, main zone, or Zone 2 |
| `set_source` | Write | A source from the catalog |
| `set_volume` | Write | Exact relative dB level, or `min` |
| `set_mute` | Write | On or off |
| `set_sound_mode` | Write | A receiver sound mode |
| `cancel_operation` | Write | Own operation, before it reaches the session |

Write tools return promptly. Agent hosts apply request timeouts that the server
cannot learn, and a human decision takes far longer. A write that needs approval
therefore returns `awaiting_approval` with an operation id, and the agent polls
`get_operation`. Any wait inside a tool call is bounded by a server-side cap
well below any plausible host timeout. Write tools accept an optional
agent-chosen request id that becomes the idempotency key.

Every write result carries the lifecycle `status`, `dispatch`, and `confirmed`
fields defined above, the reasons for any denial or hold, and the receiver
observation when one exists. Results use structured content with a plain-text
rendering. `awaiting_approval`, `completed`, `already_in_state`, and `cancelled`
are ordinary results; every other status is returned with `isError` set, with
the status and reason in the content. The text states plainly when nothing was
dispatched, when a command was sent but is unconfirmed, and that the agent must
not retry while an operation is pending.

### stdio build (`apps/mcp-stdio`)

For an agent on the Control API server's machine.

- **Launch and lifetime.** The agent host starts it as a child process from its
  MCP configuration and exchanges newline-delimited JSON-RPC over the child's
  standard streams. One process serves one agent session. It exits promptly when
  standard input closes and starts no child processes. The Control API server
  holds all durable state, so a new process can continue with `get_operation`
  for the same Agent token.
- **Credential.** The MCP specification has stdio servers take credentials from
  the environment rather than from its HTTP authorization framework. The
  preferred form is an environment variable naming an owner-only token file in
  the agent account's own directory, because an environment variable's value is
  visible to other processes of the account and inherited by every process the
  agent host starts. A token held directly in an environment variable is
  accepted for hosts that cannot do otherwise. The token never appears in
  command-line arguments, logs, or tool results. It grants nothing beyond Agent
  authority.
- **Standard streams.** Standard output carries MCP messages and nothing else;
  a stray write would corrupt the protocol. Diagnostic logging goes to standard
  error, with tokens redacted. A lint or boundary rule forbids writing to
  standard output outside the transport.
- **No network.** The package has no listener, no TLS, and no network client.
  `api-client` speaks only local transports, and the boundary check inspects this
  package's own resolved dependency graph for TLS libraries and HTTP server
  frameworks.
- **Account requirement.** The account that runs the agent host must differ
  from the owner's account, with access limited to the Agent endpoint, as set
  out in [Choosing a build](#choosing-a-build). The stdio build is the safest
  choice for a same-machine agent only under that requirement.

### Streamable HTTP build (`apps/mcp-http`)

For agent hosts on other machines, including virtual machines.

- **Binding.** The endpoint listens on one configured address. A wildcard
  address is refused unless the configuration explicitly allows it. If the
  address is not assigned yet, as with a virtual-machine bridge that exists only
  while the machine runs, the server retries with backoff and never falls back
  to another address.
- **TLS.** The endpoint refuses to start without TLS, which it terminates itself
  using a certificate and key the user provides (a private CA or self-signed
  certificate that the agent host trusts). The certificate must cover the
  address or name the agent host connects to. A client certificate (mutual TLS,
  for agent hosts that can present one) and a source-address allowlist can be
  required in addition.
- **Authentication.** There are two credential modes, each enabled explicitly
  in configuration; the default is static tokens only. In both, every request
  carries its credential in an `Authorization: Bearer` header and a token in a
  query string is rejected. A request with no credential is refused without
  contacting the Control API. Failed authentication is rate-limited per source
  address. No session identifier is ever a credential, and each request is
  authenticated on its own. When both modes are enabled, static tokens carry a
  fixed, recognizable prefix so the two kinds are told apart by format, and a
  credential that fits neither is refused.
- **Static-token mode.** The Agent token is the Control API server's own
  credential for the Agent endpoint. The server forwards it unchanged and treats
  a Control API rejection as authoritative. This mode sits outside the MCP
  specification's OAuth authorization framework, which HTTP transports are
  expected to follow; the deviation is deliberate and recorded here.
- **OAuth mode.** `mcp-http` is an OAuth 2.1 resource server. It publishes
  Protected Resource Metadata that names the authorization server and answers a
  missing or invalid token with 401 and a pointer to that metadata. It accepts
  only tokens whose audience is its own canonical resource URI, and it never
  forwards the inbound token, because the specification forbids passing a
  client's token on to another service. To call the Agent endpoint it exchanges
  the inbound token at the authorization server (token exchange, RFC 8693) for a
  separate short-lived token whose audience is the Agent endpoint and which names
  the same agent client. The Control API server therefore still derives the
  principal itself from a signed token, and `mcp-http` stays an untrusted
  adapter. Exchanged tokens may be cached in memory until shortly before they
  expire; the cache is an optimization the server does not rely on. The stdio
  build does not use OAuth: the specification has stdio servers take credentials
  from the environment.
- **Request validation.** A request carrying an `Origin` header that is not
  explicitly allowed receives 403, as the MCP specification requires for an
  invalid origin; agent clients are not browsers, so the allowed set is empty by
  default. The `Host` header is checked against an allowlist. Request body size,
  concurrent connections, and in-flight requests per source and per token are
  capped, and header and idle timeouts apply.
- **No per-client state.** Tools return promptly and the server sends no
  unsolicited messages, so replies are plain JSON rather than long-lived
  streams, and the server keeps no state it relies on between requests.
- **Credential limits.** In static-token mode the server holds no secret other
  than its TLS key and sees each caller's Agent token in transit. In OAuth mode
  it also holds a client credential of its own for the exchange. That credential
  is useless without a valid inbound token: the server cannot mint or elevate a
  credential for an agent whose token it has not been given.
- **Agents that speak only stdio on another host.** The stdio build cannot
  serve them; see [Choosing a build](#choosing-a-build). Such a host needs a
  stdio-to-HTTP bridge of its own, on its own machine, holding that agent's
  token and pointing at this endpoint. The bridge belongs to that agent host and
  is not part of this repository.

### Reference deployment: an agent in a UTM guest

An OpenClaw install in a UTM Linux guest on a macOS host is a remote agent host,
so it uses `mcp-http`. Background and unverified details are in
[UTM guest LAN isolation](research/utm-guest-lan-isolation.md) and
[OpenClaw user and root risk](research/openclaw-vm-user-and-root-risk.md).

- The endpoint binds the vmnet gateway address that the guest uses to reach the
  host. That address exists only while the guest runs, which is the retry case
  above. A wildcard bind is not used: the research notes that host services bound
  to `0.0.0.0` are reachable from the guest anyway, and a wildcard bind would
  also expose the endpoint to the whole LAN.
- The research's host `pf` rule set blocks everything on the gateway except
  DNS and DHCP. The MCP port needs an explicit pass rule ahead of the block, and
  that is the only opening to the guest. This has not been tested. UTM's
  "Isolate Guest from Host" setting would block the endpoint and cannot be used.
- When OAuth is enabled, the authorization server is a second endpoint the
  guest must reach. It needs the same treatment as the MCP endpoint: its own
  bound address, TLS, and an explicit `pf` pass rule for its port.
- Root in the guest can read the agent's token or OAuth credentials, which grant
  only Agent authority; revoke the token or the client registration if the guest
  is compromised.
- Validation follows the research documents: the guest reaches the MCP port and
  cannot reach the receiver, recorded as pass or fail after every reboot,
  Wi-Fi change, and macOS update.

## GUI app and CLI

**GUI.** The existing Iced presentation (`gui-lib`, `apps/desktop`) is
retargeted to be an Operator client. The first release supports macOS only: the
macOS visual baselines are updated, the Windows and Linux baselines are left as
they were and are not validated, and the macOS app bundle is a packaged
distribution of it. It projects presentation state from receiver state and
operation events and adds views for the operation feed, pending approvals
(observe and cancel), the audit log, and the effective policy (read-only, with
its digest and a reload action). It authenticates with the Operator token file
on the Operator endpoint. It neither issues nor revokes agent credentials; the
CLI manages those. The GUI sees only the control-service port, so its toolkit
is not an architectural commitment.

**CLI.** The CLI becomes an Operator client with the same `get` and `set`
surface and authenticates with the Operator token file. `--dry-run` becomes a
server-side policy evaluation that reports the decision a real request would
receive. A new command group issues, lists, and revokes Agent tokens. Direct
session construction is removed from the CLI: a CLI that opened its own receiver
session would be a second session owner whose writes are neither serialized with
the others nor audited.

**Diagnostics.** Diagnostics keep opening their own connections and keep their
read-only guarantee. They are an evidence tool, not a control path.

## Security model

Principals and trust:

| Party | Trust |
| --- | --- |
| Control API server | Trusted; the only holder of receiver access, policy, audit, and approval verification keys |
| Operator clients (GUI, CLI) | Trusted to speak for the user |
| Any process that can read the Operator token file | Acts as Operator; accepted, see below |
| MCP servers (either build) | Untrusted adapters, limited to the Agent endpoint and the Agent principal |
| Authorization server | Trusted to mint Agent credentials only; never to grant Operator authority |
| AI agents and the models behind them | Untrusted; may act on hostile content they have read |
| External Approval Service | Trusted only for signed decisions about tickets this server issued |

Threats and controls:

| Threat | Control |
| --- | --- |
| Agent, misled by hostile content, requests a harmful change | Policy Engine classification at the gate; dangerous operations need approval; hard limits deny |
| An agent, or several agents, split a large change into small steps | Cumulative budget input to policy, pooled across agents |
| Agent approves its own request | No agent-reachable approval path; decisions require the external service's signature |
| Approval prompt does not match the executed action | Server-generated summary; digest binds receiver, intent, baseline, and expiry; re-evaluation at dispatch; precondition checked by the session |
| Replayed or stale approval | Single-use nonce, expiry on the server's clock, ticket state |
| State changes while the user decides | Baseline in the digest; re-evaluation; a reconnect or receiver change voids the approval |
| An agent floods approval requests | Per-client pending and rate caps; bounded re-approval |
| Client retry causes a double write | Server-side ids, idempotency, coalescing of identical in-flight operations, existing at-most-once dispatch |
| Defective MCP server issues writes | Neither build has a receiver path; both can reach only the Agent endpoint and Agent authority |
| Compromised `mcp-http` process reads the Operator token | Prevented only when it runs under a dedicated account without access to the owner's data directory; otherwise accepted |
| Authorization server compromised or its signing key stolen | Yields Agent credentials only, never accepted on the Operator endpoint. The key is readable only by the server's own account, tokens are short-lived, and a token is honored only while its client is registered at the Control API server, so revoking the registration ends access |
| Token confusion between resources | `mcp-http` accepts only tokens whose audience is itself, the Agent endpoint accepts only tokens whose audience is the Agent endpoint, and an inbound token is never forwarded |
| Network-facing authorization server is abused | TLS, one bound address, rate limits, and client credentials kept out of URLs and logs, as for `mcp-http` |
| A command-running agent uses the CLI or the Operator token | Out of reach when the agent runs as a different account or on a different machine; not prevented under the owner's account |
| Same-machine agent reaches the receiver directly | Not prevented by software; an operating-system rule matching the agent's account is needed and is unvalidated |
| Agent host reaches the receiver directly over the LAN | Not prevented by software; network rules must block it (see below) |
| Another LAN host uses or sniffs the MCP HTTP endpoint | TLS is mandatory; bearer token; optional client certificate and source-address allowlist; rate-limited failed authentication; one bound address |
| A web page drives the HTTP endpoint through a browser (DNS rebinding) | `Origin` and `Host` validation, with 403 on an invalid origin |
| Token leaks through a URL, log, or process listing | Header-only tokens; redacted logs; the stdio token comes from an owner-only file rather than the command line |
| Stray output corrupts the stdio protocol stream | Standard output carries MCP messages only; logs go to standard error |
| MCP path reads or changes policy or credentials | The Agent endpoint serves no Operator resource; stored tokens are hashes and the approval keys are public; policy file edits take effect only on server start or an Operator reload, and the digest of the loaded policy is audited |
| Tampering with policy or approval configuration | Not writable from any agent surface; load failure disables agent writes |

Fail-closed behavior:

| Condition | Behavior |
| --- | --- |
| Policy missing or invalid, at start or on reload | Agent writes end `rejected` with nothing dispatched; reads continue |
| Receiver epoch not established, or the audit log unreadable when the ledger is rebuilt | Agent writes end `rejected` with nothing dispatched |
| Approval service unconfigured or unreachable | Approval-requiring operations end as `approval_unavailable` |
| Baseline state stale or unknown for a state-dependent rule | Treated as dangerous |
| Intent without a classification | Treated as dangerous |
| Audit sink failing | Agent writes rejected; Operator controls continue with a prominent warning so a fault never strands the user |
| Agent endpoint permissions cannot be applied | The Agent endpoint is not created; the Operator endpoint still serves |
| TLS material missing or invalid | `mcp-http` does not start |
| OAuth mode enabled but its configuration is incomplete | `mcp-http` does not start |
| Authorization server unreachable or its public keys unavailable | OAuth-authenticated requests fail; static-token requests and the Operator endpoint are unaffected |
| Server restart | Pending operations expire; no operation resumes |

Scope of protection. The Policy Engine and approval flow guard the MCP path.
The MCP servers, GUI, and CLI cannot construct a receiver session, and the Agent
endpoint cannot reach Operator resources. The design depends on deployment
assumptions that software here cannot enforce:

- The agent runs on a different machine, or under a different operating-system
  account that cannot read the owner's data directory. A process that can run
  commands as the owner's account could read the Operator token and act as
  Operator, run the CLI, or edit the policy files. Under the owner's account
  policy binds only an agent restricted to MCP tools.
- Network rules (firewall or VLAN) let a remote agent host reach the MCP
  endpoint but not the receiver, and an operating-system rule keeps a
  same-machine agent account from the receiver. The receiver's own protocols are
  unauthenticated, so an agent that can reach them has a path around everything
  in this document.
- The agent host's own command controls, such as command approval or sandboxing,
  remain relevant, and the design does not claim to replace them.

Within those assumptions the agent's only route to the receiver is a gated
operation.

## Workspace and dependencies

New packages are `policy`, `api-contract`, `api-client`, `mcp-tools`,
`apps/api-server`, `apps/mcp-stdio`, `apps/mcp-http`, and `apps/auth-server`.
Permitted workspace edges:

```text
protocol       → domain
policy         → domain
application    → domain, policy
api-contract   → application, domain
api-client     → api-contract, application, domain
infrastructure → application, domain, policy, protocol
gui-lib        → application, domain
cli            → api-client, application, domain
desktop        → api-client, application, domain, gui-lib
mcp-tools      → application, domain
mcp-stdio      → api-client, application, domain, mcp-tools
mcp-http       → api-client, application, domain, mcp-tools
auth-server    → api-contract
api-server     → api-contract, application, domain, infrastructure, policy
diagnostics    → domain, infrastructure, protocol
```

`api-contract` also defines the access-token claims, the client-registration
schema, and the registry export that `auth-server`, `api-server`, and `mcp-http`
share.

`make boundary` must enforce, in addition to today's rules:

- Only `infrastructure`, `api-server`, and `diagnostics` may depend, directly or
  transitively, on `protocol`, and only `api-server` and `diagnostics` may
  depend on `infrastructure`. This makes "the agent-facing process cannot reach
  the receiver" a property of the resolved Cargo graph.
- `policy` stays pure: no async runtime, serialization, filesystem, network,
  or clock reads. Time arrives as an input.
- `mcp-tools` contains no transport implementation, and `api-client` has no TLS
  dependency and speaks only local transports.
- The resolved graph of `mcp-stdio`, taken for that package alone so that
  feature unification across the workspace cannot add to it, contains no TLS
  library and no HTTP server framework. The script holds the list.
- `mcp-stdio` and `mcp-tools` do not write to standard output.
- The gate is the only caller of the session's `operate` outside tests and the
  concrete session implementation.
- Receiver sessions are constructed only in `api-server` and `diagnostics`.
- The edge allowlist and the session-contract definition rule in the boundary
  script are updated with the graph above and the contract change described
  under [Receiver core and the session contract](#receiver-core-and-the-session-contract).

## Configuration, credentials, and runtime

| Item | Location and rule |
| --- | --- |
| Receiver configuration | YAML, owned by the Control API server and reached by clients through the API. The schema holds several receivers by name. An earlier single-receiver file is read, then rewritten in the new schema with a one-time backup |
| Policy | `policy.yaml`, a separate YAML file beside the receiver configuration in the per-user data directory, edited as a file and loaded on start or Operator reload |
| Audit log | Append-only JSON Lines in an `audit` directory beside the configuration (directory mode 0700, files 0600); one record per operation event, as described under [Audit](#audit). Rotated by size, with a configurable size limit and file count (defaults 20 MiB and 10 files); the oldest file is deleted beyond the count, and the audit view reads across the retained files |
| Endpoints | The Operator endpoint in the owner-only data directory; the Agent endpoint in a separate location whose permissions are configured and applied at start |
| Credentials and keys | Owner-only files in the per-user data directory: hashes of issued tokens, the Operator token file read by the GUI and CLI, and the External Approval Service's public keys with their key ids |
| Agent tokens | Held by the agent side only: in the agent host's MCP configuration as a request header for `mcp-http`, or in an owner-only file named by the environment for `mcp-stdio` |
| TLS certificate and key | `mcp-http` only; readable by the account that runs it and no other |
| OAuth client registrations | Held by the Control API server: agent label, client id, and revocation state, managed through the token routes. The server also writes a read-only export of the registered clients that the authorization server's account can read |
| Authorization server keys | The token signing key and the server's TLS key, readable only by the authorization server's account. Its public keys, with key ids, are in the Control API server's data directory |
| Exchange credential | OAuth mode only; `mcp-http`'s own client credential at the authorization server, readable by its account and no other |
| Diagnostic logging | Existing `tracing` logs, kept separate from the audit log; `mcp-stdio` logs to standard error only |

The audit log is for accountability and incident review. It is not tamper-proof
against a process running as the same user.

## Verification

Verification follows the existing policy in [contributing.md](contributing.md)
and [development.md](development.md): deterministic fakes for ordinary tests
and opt-in live validation.

- The Policy Engine is tested as a pure function: table cases per rule, and
  generated properties. Evaluation is deterministic. Raising a volume target
  never lowers severity. An unknown or stale baseline never yields `Allow` for
  a state-dependent rule. A `Deny` match is never weakened by an `Allow`.
  Alternating up and down steps never resets the budget.
- The gate is tested with a fake session, fake approval channel, and fake clock:
  approve, reject, expire, cancel, replay, mismatched digest, state change
  during approval, a failed precondition, supersession of an approved operation,
  retry with the same idempotency key, two agents splitting a budget
  concurrently, and an audit sink that fails before dispatch. Each asserts at
  most one dispatch and no dispatch on any non-approved path. Every session outcome
  is checked against the mapping to `status`, `dispatch`, and `confirmed`.
- The Control API has contract tests against the schema and an in-process
  server. The Agent endpoint is tested to refuse every Operator resource, with
  and without an Operator token, and to audit the attempt. Receiver addresses
  are absent from the Agent view.
- One shared conformance suite for `mcp-tools` runs against both builds with an
  in-process MCP client over each transport and a fake control-service port. It
  asserts that no write tool reports success without evidence and that
  ownership follows the credential.
- With OAuth enabled, `mcp-http` is tested for rejecting with 401 a token of
  the wrong audience, an expired token, and an unsigned one; for serving its
  Protected Resource Metadata; and for never forwarding the inbound token, using
  a fake Control API that records the credential it receives. The Agent endpoint
  is tested for rejecting an authorization-server token with the wrong audience,
  an expired one, or one whose client is revoked, and for rejecting every such
  token on the Operator endpoint. The authorization server is tested for
  issuance, expiry, and refusing an exchange without a valid subject token.
- The stdio build is also tested for emitting only valid MCP messages on
  standard output, exiting when standard input closes, starting while the
  Control API server is down, and never printing its token.
- The HTTP build is also tested for refusing to start without TLS, refusing a
  wildcard bind without explicit opt-in, retrying an unassigned address, rejecting
  missing or invalid tokens, rejecting a token in a query string, answering 403
  to a disallowed `Origin`, enforcing the body and connection caps, and
  rate-limiting failed authentication.
- Live receiver validation remains opt-in and records the same evidence as
  today. Deployment checks for the network and account assumptions are recorded
  like the research documents' validation plans.

## Alternatives considered

| Alternative | Why it was not chosen |
| --- | --- |
| GUI app hosts the server | Agent availability would depend on a window's lifecycle and crash domain, and the GUI would become the enforcement point |
| MCP server embeds the receiver session and policy | A second session owner, and a defect in the network-facing adapter would also defeat enforcement |
| Policy evaluated inside the MCP server | Enforcement would live outside the process that owns the session, and the CLI and GUI would not share it |
| Relying on MCP annotations or host approval prompts | Client-controlled and may be disabled or keyed by tool rather than arguments |
| MCP elicitation as the approval channel | Routes approval through the host the policy is guarding against |
| Approval inside the GUI as well as the external service | Two channels double the trust surface; the GUI observes and cancels only |
| CLI keeps direct sessions | A second session owner with unaudited writes, and a policy bypass that needs no credential at all |
| GUI policy editor | A rule editor to build and secure for little gain over a validated file; keeping policy unwritable through the API leaves no route for an agent to change it |
| Block in the tool call until the user decides | Exceeds host request timeouts and invites retries |
| Streamable HTTP only | Gives a same-machine agent a network listener it does not need |
| stdio only | Cannot serve agent hosts on other machines or in virtual machines |
| One MCP binary with a runtime transport flag | The stdio deployment would still carry TLS and listener code. Two builds make "no network surface" a property of the dependency graph |
| stdio build on a remote agent host, reaching the Control API over the network | Exposes the whole Control API to the network. Remote hosts use the HTTP build, and a stdio-only remote host bridges to it |
| `mcp-http` forwards the inbound OAuth token to the Control API | The MCP specification forbids passing a client's token on to another service |
| `mcp-http` asserts the calling agent with its own credential | A compromised `mcp-http` could act as any agent and defeat per-agent attribution and revocation. Token exchange keeps the principal derived from a signed token |
| Authorization server inside `mcp-http` | The network-facing process would hold the signing key, contradicting its untrusted-adapter role |
| Authorization server inside the Control API server | The Control API server has no network listener, and agent hosts must reach the authorization server |
| One Control API endpoint, with Agent and Operator told apart by token alone | An agent-side account would reach the Operator surface behind a single token check. Two endpoints make Operator resources absent from the Agent endpoint and let the operating system admit a different account to it |

## Open decisions

1. **Approval wire contract.** The stream protocol (server-sent events or a
   WebSocket), the cursor semantics for resuming, the public-key algorithm,
   key distribution and rotation, and the signed encoding must be settled with
   the design of the External Approval Service, which the project owner is
   building separately. The server only makes outbound connections and verifies
   signatures with public keys.
2. **Agent endpoint access.** The mechanism that admits a dedicated agent or MCP
   service account to the Agent endpoint on macOS (group membership or an access
   list), with other systems deferred, and what the server verifies about the
   peer.
3. **MCP protocol revision and SDK.** The revision to target, and whether to
   adopt the official Rust SDK, which publishes both a stdio transport and a
   Streamable HTTP server with host, origin, and body-size settings behind
   separate feature flags. The two-build split keeps each build's features
   apart. Revision 2026-07-28 defines a mandatory `server/discover` method that
   returns capabilities and instructions; its session semantics were not
   verified here, and stateless operation of the HTTP build depends on them.
4. **OAuth details.** Decided: static tokens stay, an optional OAuth mode is
   added, a separate authorization server issues the tokens, and `mcp-http` uses
   token exchange. Still open: the grant a headless agent uses (a pre-registered
   client with no user interaction, or authorization code with PKCE and a consent
   page; whether the targeted revision permits the former was not verified); how
   clients are identified (Client ID Metadata Documents, which the specification
   recommends, or pre-registration, since Dynamic Client Registration is
   deprecated); token format, lifetimes, and key rotation; the exchange
   details; the format and refresh of the registry export; and the account and
   address the authorization server runs under.
5. **Same-machine receiver isolation.** Whether and how an operating-system
   firewall rule matching the agent's account is validated and recorded, for
   macOS first.
6. **Session-contract precondition.** Closed: implemented, and described in
   [Control service](../ARCHITECTURE.md#control-service).

## References

MCP specification revision 2026-07-28, read 2026-10-07:
[stdio transport](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/stdio),
[Streamable HTTP transport](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http),
[authorization](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization),
[authorization server discovery](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/authorization-server-discovery),
and [authorization security considerations](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/security-considerations).
The OAuth standards named in the text (RFC 8414, RFC 8707, RFC 9728) are cited by
those pages and were not read separately; RFC 8693 token exchange is cited from
general knowledge and must be checked before it is designed in detail. The Rust
SDK's published API documentation is at
[docs.rs/rmcp](https://docs.rs/rmcp/latest/rmcp/).
