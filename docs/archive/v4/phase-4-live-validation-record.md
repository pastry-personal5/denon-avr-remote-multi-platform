# Version 4, Phase 4 — Control API server, contract, and client: validation record

**Not run.** Every check below writes to a receiver, needs the Mac's real file
permissions and the dedicated agent account, or both, and the session that
implemented the phase had neither. The phase's code is on branch
`v4/phase-4-control-api-server` and passes `make check`, `make clippy`, and
`git diff --check`. This record is the owner's to fill; until its **Result** column
is complete, the phase's live exit criteria are open and the branch is not to be
merged.

The armed live test was written and compiled, and its Makefile target was checked to
refuse when unarmed. It has **not been run against a receiver**.

**Step 9 is not built.** The plan makes reading the Agent endpoint's directory and
admitted uids from `server.yaml` wait for S2, the hand check below, because S2 fixes
that file's keys and the default directory and is the moment an agent can first
connect to a real server. Until then the executable serves the Operator endpoint
alone; the Agent endpoint is built, tested, and exercised by the live test, which
gives the library its directory and uid as values.

The behaviors being checked are in the
[architecture](../../v4/phase-4-control-api-server-architecture.md) and, now that
they are implemented, in [ARCHITECTURE.md](../../../ARCHITECTURE.md#control-api).
The exit criteria are in the
[overview](../../v4/phase-4-control-api-server-overview.md#exit-criteria).

## Before running

1. **Network, once.** The phase adds `axum`, `hyper`, `hyper-util`, `http-body-util`,
   `bytes`, `futures-util`, `getrandom`, and `subtle`, and what they bring. They are
   in `Cargo.lock`, so a build with network access fetches them once. Run
   `cargo fetch` with network access before a sandboxed `make check`.
2. **Sockets.** The test suite binds Unix sockets and loopback ports. Run
   `make check` outside a sandbox that blocks them. The server tests make their
   socket directories under `/tmp` so the paths stay under 104 bytes.
3. **A level you accept.** Decide the highest volume you are willing for the test to
   name, in half decibels (`-40` is -20.0 dB), and export it as
   `DENON_X3800H_SAFE_VOLUME_HALF_STEPS`. The test builds its limits from the volume
   the receiver shows now, `L`, and refuses to write anything unless its highest
   target, `L + 3.5 dB`, is at or below that level.
4. **No competing client.** The receiver accepts one control connection. Close the
   GUI and the CLI before the live run, and do not run them while a server holds
   the receiver.

## S2: the Agent endpoint's directory and admission

Run on the Mac with the dedicated agent account, following the procedure in the
[architecture](../../v4/phase-4-control-api-server-architecture.md#s2-what-the-hand-check-decides).
Its output is `docs/research/agent-endpoint-access-macos.md`, which fixes the
`agent_endpoint` keys and the default directory and is what step 9 waits for.

| Check | Expected | Result |
| --- | --- | --- |
| The agent account can `connect()` to a socket in the chosen directory, by an ACL or by a shared group, and the mode the socket needs | recorded | To fill |
| `/Users/Shared/<name>` with the `lstat` and owner checks, or another location | decided | To fill |
| `peer_cred()` reports the agent account's uid, for a process started with `sudo -u` and for one started by the agent host | recorded | To fill |
| The agent account cannot open `credentials/`, `run/`, or the audit directory, and cannot connect to the Operator socket | true | To fill |
| The access still holds after the server restarts and the socket is recreated | true | To fill |

## Permissions after a run

Start the server with a data directory of your own, then:

```text
ls -ld <data>/run <data>/credentials
ls -l  <data>/run <data>/credentials
```

| Check | Expected | Result |
| --- | --- | --- |
| `run/` and `credentials/` | `drwx------` | To fill |
| `operator.sock`, `server.lock`, `operator.token`, `agent-tokens.json` | owner-only (`-rw-------`, the socket `srw-------`) | To fill |
| The data directory itself | unchanged from before the run | To fill |

## The account boundary (after step 9)

As the dedicated account, with an Agent token the Operator issued:

| Check | Expected | Result |
| --- | --- | --- |
| A request to the Agent socket with the Agent token | succeeds | To fill |
| Connecting to `operator.sock` | fails | To fill |
| Listing `run/`, `credentials/`, and the audit directory | fails | To fill |
| Reading `operator.token` | fails | To fill |
| The Operator token presented on the Agent socket | `401`, and an `access_refused` record in the audit log | To fill |

## A killed server

| Check | Expected | Result |
| --- | --- | --- |
| `kill -9` the server, then start it again | the second start binds and serves | To fill |
| Start a second server while one runs | the second exits with status 75 and the first is unaffected | To fill |

## Live run

```text
ALLOW_RECEIVER_WRITES=1 \
DENON_X3800H_HOST=<the receiver> \
DENON_X3800H_SAFE_VOLUME_HALF_STEPS=<your level> \
make test-live-x3800h-api
```

The test starts the server in its own process on a temporary directory with the real
connector, a temporary policy, and a temporary audit directory, issues an Agent token,
and drives the receiver through `ApiClient`. With `L` the volume it reads:

| Step | Expected | Result |
| --- | --- | --- |
| The test refuses to write when `L + 3.5 dB` is above the declared safe level | refused before any write | To fill |
| Operator `get state` and `refresh` | the receiver's current state; nothing written | To fill |
| Agent: volume to `L + 1.0 dB` | `completed`, `complete_write`, and the receiver shows it | To fill |
| Agent: volume to `L + 2.0 dB` | `approval_unavailable`, receiver unchanged | To fill |
| Agent: volume to `L + 3.5 dB` | `denied`, receiver unchanged | To fill |
| Operator: volume restored to `L` through the Operator endpoint | `completed`, receiver back at `L` | To fill |
| The audit log holds `Decided`, `Dispatching`, and `Finished` for the change, and `Decided` and `Finished` for each refusal | present | To fill |
| No audit record holds a token | true | To fill |
| `credentials/` and the audit directory | `drwx------`, files `-rw-------` | To fill |
| The receiver's volume after the test | `L` | To fill |

Record the receiver's model and firmware, `L`, the safe level you declared, the
command, and the date.
