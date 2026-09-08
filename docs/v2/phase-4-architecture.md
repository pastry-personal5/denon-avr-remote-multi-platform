# Version 2 - Phase 4 Architecture

```text
CLI / GUI presentation
          |
    ReceiverController ---- configuration / discovery
          |
    ReceiverSession -------- AVR session / transport
          |
    typed domain snapshots and controls
```

`ReceiverController` is an application policy boundary. Selection accepts
saved identities, discovered identities, and explicit hosts. Selection changes
invalidate the snapshot and increment its resource version. A session factory
creates a typed session; the controller never parses or constructs protocol
strings.

The async actor/channel facade is intended for long-lived callers and runs on
a caller-owned Tokio runtime. One-shot callers use the same admission rules
and leave confirmation explicitly deferred after a single dispatch.

Each status field remains independently available or unavailable. Accepted
observations advance the resource version, including changed fields in a
partial result. Control dispatch is never retried. Read-only queries may be
retried by the session, while reconnect and power-on quiet time are bounded.

Observability receives typed, redacted diagnostics for generations, reconnects,
timeouts, malformed frames, queue pressure, control confirmation, and
shutdown. It cannot alter policy.
