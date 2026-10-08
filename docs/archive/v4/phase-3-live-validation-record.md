# Version 4, Phase 3 — Policy, audit, and the Agent path: validation record

**Not run.** Every check below writes to a receiver, needs the Mac's real file
permissions, or both, and the session that implemented the phase had a receiver
for neither. The phase's code is on branch `v4/phase-3-policy-audit-gate` and
passes `make check`, `make clippy`, and `git diff --check`. This record is the
owner's to fill; until its **Result** column is complete, the phase's live exit
criteria are open and the branch is not to be merged.

The armed live test was written and compiled, and its Makefile target was checked
to refuse when unarmed. It has **not been run against a receiver**.

The behaviors being checked are in the
[architecture](../../v4/phase-3-policy-audit-gate-architecture.md) and, now that
they are implemented, in [ARCHITECTURE.md](../../../ARCHITECTURE.md#policy-and-audit).
The exit criteria are in the
[overview](../../v4/phase-3-policy-audit-gate-overview.md#exit-criteria).

## Before running

1. **Network, once.** The phase adds `serde_json` and `sha2`. They are in
   `Cargo.lock`, so a build with network access fetches them once. Run `cargo fetch`
   with network access before a sandboxed `make check`.
2. **Loopback.** The test suite binds loopback sockets. Run `make check` outside a
   sandbox that blocks local ports, or allow local binding for it.
3. **A level you accept.** Decide the highest volume you are willing for the test
   to name, in half decibels (`-40` is -20.0 dB), and export it as
   `DENON_X3800H_SAFE_VOLUME_HALF_STEPS`. The test builds its limits from the
   volume the receiver shows now, `L`, and refuses to write anything unless its
   highest target, `L + 3.5 dB`, is at or below that level.

## Operator regression

The phase changes nothing the Operator sees, and the CLI and the GUI still build
their service with `ControlService::new`. Repeat the phase 1 live checks to prove
it, with the model, firmware, settings, commands, responses, and date recorded as
before.

| Check | Expected | Result |
| --- | --- | --- |
| Read-only `get status` through the CLI | the receiver's current state | To fill |
| `make test-live-x3800h-controls`, armed | passes and restores the receiver | To fill |
| Model, firmware, relevant settings, date | recorded | To fill |

## Agent run

```text
ALLOW_RECEIVER_WRITES=1 \
DENON_X3800H_HOST=<the receiver> \
DENON_X3800H_SAFE_VOLUME_HALF_STEPS=<your level> \
make test-live-x3800h-agent
```

The test starts a service with the real connector, a temporary policy file, and a
temporary audit directory, and uses an Agent handle. With `L` the volume it reads:

| Step | Expected | Result |
| --- | --- | --- |
| The test refuses to write when `L + 3.5 dB` is above the declared safe level | refused before any write | To fill |
| Volume to `L + 1.0 dB` | `completed`, `complete_write`, and the receiver shows it | To fill |
| Volume to `L + 2.0 dB` | `approval_unavailable`, receiver unchanged | To fill |
| Volume to `L + 3.5 dB` | `denied`, receiver unchanged | To fill |
| Volume restored to `L` by the Operator | `completed`, receiver back at `L` | To fill |
| The audit log holds `Decided`, `Dispatching`, and `Finished` for the change, and `Decided` and `Finished` for each refusal | present | To fill |
| The receiver's volume after the test | `L` | To fill |

Record the receiver's model and firmware, `L`, the safe level you declared, the
command, and the date.

## Audit permissions

The test prints the audit directory it used and leaves it in place. Afterwards:

```text
ls -ld <the directory>
ls -l  <the directory>
```

| Check | Expected | Result |
| --- | --- | --- |
| The directory | `drwx------` | To fill |
| `audit.jsonl` | `-rw-------` | To fill |
| Each line is one JSON object, and no line holds text an agent chose beyond the bounded intent | true | To fill |

## The owner's policy

Nothing in this phase can catch a misspelt agent label, because tokens do not
exist until milestone 4: labels match exactly, case and all, so `Claude-Code` in a
rule leaves `claude-code` under the general rules. `dry_run_as` is how a tier is
checked. Through a test or the in-process port, ask what the policy decides for
the label you mean.

| Check | Expected | Result |
| --- | --- | --- |
| `dry_run_as` with each agent label the policy names, for a volume change and a power change | the decision the policy's author intended | To fill |
| `reload_policy` after an edit that is wrong | the policy is unavailable, agent writes are `rejected`, and the Operator's controls still work | To fill |
| `reload_policy` after the edit is put right | agent writes are decided again | To fill |
