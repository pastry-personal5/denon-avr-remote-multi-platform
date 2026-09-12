# Version 1 - Phase 2 Architecture

Phase 2 introduces a persistent session actor above the AVR wire primitives.
One reader owns the Tokio TCP socket, one serialized request path owns writes,
and unrelated CR-delimited lines are routed as raw events. Connection,
timeout, disconnect, malformed-frame, unexpected-response, and receiver-error
conditions are represented separately.

After disconnect, reconnect attempts are finite and use configurable backoff.
A generation or lifecycle transition invalidates cached authority; the
application must issue a fresh status query instead of trusting missed events.
The synchronous `AvrTransport` and `TcpAvrTransport` path remains operational
for compatibility.

## Current architecture carried by Version 1

The completed refactoring adds `application.rs` for receiver policy,
`transport.rs` for the runtime-neutral async contract, `response.rs` for
shared correlation and field parsing, and `state.rs` for evidence-backed
typed main-zone reduction. `PW`, `SI`, `MV`, `MU`, and `MS` events are typed;
unknown lines remain raw. Serde YAML validates configuration and saves through
a sibling temporary file before replacement. Legacy direct transport APIs are
deprecated but retained until a separately planned removal release.

No typed support is added for unvalidated zones or telemetry. Capability
claims remain evidence-bound to the X3800H validation record.
