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

**S2 was run by the owner on 2026-10-09**, and step 9 followed it: the executable reads
the Agent endpoint from `server.yaml`. The S2 results are in
[agent-endpoint-access-macos.md](../../research/agent-endpoint-access-macos.md). What is
left is the account boundary against the real server, the permissions after a run, the
killed-server check, and the armed live run.

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

Run by the owner on 2026-10-09 on macOS 26.7.1 (25G241), with the dedicated account
`agent0000` (uid 503) and a throwaway script, following the procedure in the
[architecture](../../v4/phase-4-control-api-server-architecture.md#s2-what-the-hand-check-decides).
The full results are in
[agent-endpoint-access-macos.md](../../research/agent-endpoint-access-macos.md).

| Check | Result |
| --- | --- |
| The agent account can `connect()` to a socket in the chosen directory, and the mode the socket needs | Pass. A directory ACL (`search`, and an inheritable `write`) admits it with the socket at `0600`; with only `search` and a `0600` socket it is refused; a `666` socket was admitted by the ACL alone |
| `/Users/Shared/<name>` with the `lstat` and owner checks | Pass. `/Users/Shared/Denon AVR Remote`, made `0700` by the owner |
| `peer_cred()` reports the agent account's uid | Pass: 503, for a process started with `sudo -u agent0000 -H -i`. For a process an agent host starts: not checked |
| The agent account cannot open `credentials/`, `run/`, or the audit directory, and cannot connect to the Operator socket | Pass for the owner's home and data directory (refused). `credentials/`, `run/`, and the audit directory under it are covered by the home being `0700`; not tried against a running server |
| An account outside the ACL (`_denonmcp`) cannot connect | Pass: refused, with the socket at `666` |
| The access still holds after the server restarts and the socket is recreated | Pass |

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

## The account boundary, against the real server

The real server now serves the Agent endpoint, so the S2 result can be checked against
it. As the owner, with the directory and its two ACL entries from the S2 note in place
(`docs/examples/server.yaml` lists the commands):

```sh
DATA=/tmp/dar-s9                       # short, so the socket paths stay under 104 bytes
mkdir -p "$DATA" && cat > "$DATA/server.yaml" <<'YAML'
agent_endpoint:
  uids: [503]
YAML
cargo run -p denon-avr-api-server -- --data-dir "$DATA" &
TOKEN=$(cat "$DATA/credentials/operator.token")
# The Operator issues the agent's token; the answer's `secret` is printed once:
curl -s --unix-socket "$DATA/run/operator.sock" -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' -d '{"label":"agent0000"}' http://localhost/v1/tokens
```

Then as `agent0000`, with the token it was given in `AGENT_TOKEN`:

| Check | Expected | Result |
| --- | --- | --- |
| `curl --unix-socket "/Users/Shared/Denon AVR Remote/agent.sock" -H "Authorization: Bearer $AGENT_TOKEN" http://localhost/v1/health` | `200` | To fill |
| The same with no token | `401` | To fill |
| The same path with the agent token on `/v1/tokens`, an Operator resource | `404` | To fill |
| `curl --unix-socket "$DATA/run/operator.sock" ... http://localhost/v1/health` | fails to connect (`Permission denied`) | To fill |
| `ls "$DATA/run" "$DATA/credentials"`, and `cat "$DATA/credentials/operator.token"` | all refused (a `/tmp` data directory is not the owner's real one; repeat against `~/Library/Application Support/Denon AVR Remote`) | To fill |
| As the owner, the Operator token presented on the Agent socket | `401`, and an `access_refused` line in `$DATA/audit/audit.jsonl` (`grep access_refused`) | To fill |
| As the owner, `GET /v1/health` on the Operator socket | `server.agent_endpoint.state` is `on` | To fill |
| Make the directory group-writable (`chmod 770`) and restart the server | the Agent endpoint is off with a reason in the health response, and the Operator endpoint still serves | To fill |

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
