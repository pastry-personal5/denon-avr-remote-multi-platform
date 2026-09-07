# Version 2 - Phase 7 Architecture

Main Zone is the only supported zone in all v2 phases. Additional zones are
excluded from the controller and GUI architecture.

Phase 7 replaces the temporary GUI-owned receiver worker with an
application-owned controller built on the Phase 1 layers. Dependencies continue
to point inward: Iced and the CLI translate presentation intents into
application commands, while protocol modules remain independent of sockets,
runtimes, and operating-system services.

```text
CLI presentation             Iced presentation
          \                     /
             ReceiverController
            /    |       |     \
      discovery config transport observability
                         |
                     AVR session
```

## Controller Contract

`ReceiverController` accepts typed commands for receiver selection, connect,
disconnect, refresh, and the Phase 3 main-zone controls. It publishes lifecycle
changes, partial typed snapshots, field errors, freshness/authority transitions,
and structured control outcomes. The controller owns policy for saved identity,
discovery fallback, reconnect invalidation, confirmation, and power-on timing.

The controller is independent of Iced. The GUI maps controller updates into
view state, and v1 operations remain adapters over the same application and
domain behavior where compatible.

## Injectable Edges

Reuse the Phase 1 interfaces for discovery, configuration, and status access,
then extend the application boundary with transport/session creation, timing,
and observability needed by the long-lived controller. A no-op observability
sink is the default; alternate sinks receive structured events without
credentials, raw configuration contents, or unnecessary network identifiers.

The new interfaces are additive. The canonical application APIs, asynchronous
status gateways, and CLI entry points remain available directly.

## State Integration

Use the typed main-zone state and reducer established in Phase 1. Authoritative query results and unsolicited events continue to use the same
parsers and reducer while retaining different authority markers. The controller
must not introduce a parallel GUI-specific snapshot model containing business
rules.

Unknown, unsupported, malformed, disconnected, and temporarily unavailable
states remain distinct. Refactoring must not turn partial status into an
all-or-nothing result.

## Cancellation and Shutdown

The session receives an explicit cancellation signal. Shutdown stops accepting
new work, resolves queued queries as cancelled, classifies dispatched controls
as unconfirmed when delivery cannot be determined, closes event streams, and
waits for bounded worker termination. Reconnect backoff and power-on delay must
be cancellable without violating execute-once semantics.

## Observability

Structured events cover connection generation, reconnect attempts, bounded
timeouts, malformed frames, receiver errors, confirmation outcomes, and event
channel pressure. Observability is diagnostic only and cannot change receiver
policy or expose secrets.

## Verification Boundaries

Contract fakes and a controllable clock exercise selection, reconnect,
confirmation, cancellation, and shutdown without wall-clock sleeps. Local TCP
tests continue to verify the real session adapter. Compatibility tests pin v1
CLI output and public wrappers, while GUI tests verify that the thinner Iced
adapter preserves Phase 2 and Phase 3 behavior.
