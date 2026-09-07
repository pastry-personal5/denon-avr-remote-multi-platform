# Version 2 - Phase 2 Architecture

Phase 2 adds an Iced presentation edge above the layered application built in
Phase 1. The GUI owns display state and user intent only; it does not frame
commands, open sockets, parse receiver lines, infer capabilities, or import
infrastructure implementations directly.

The implementation targets the stable Iced 0.14 API line and uses Iced tasks
and subscriptions to communicate with a background receiver worker. Adding the
GUI as a second binary must leave an explicit default or explicit run targets
so the established CLI workflow is unambiguous.

```text
Iced view/update
      | GUI intents and immutable updates
receiver worker
      | receiver selection and status operations
application use cases and ports
      | concrete adapters
infrastructure AvrSession
      | serialized CR-framed requests and events
receiver TCP 23
```

## Receiver Worker

One background worker owns the `AvrSession`, receives connect, disconnect, and
refresh intents, and publishes structured lifecycle and snapshot updates. This
single-owner design avoids sharing the session event receiver with the Iced
view and provides the seam where Phase 6 will add control intents.

On connection, the worker queries all five main-zone fields and marks each
successful value authoritative. Per-field errors remain independent. Validated
unsolicited events pass through `protocol::avr::parse_main_zone_event` and update only the
affected field with event authority. On disconnect or reconnect, the worker
invalidates all cached authority before querying a replacement snapshot.

All discovery and network work runs outside Iced's synchronous view function.
Operations remain bounded by existing timeouts, and results are returned as
messages rather than panics or blocking UI calls. Dropping the GUI channels
ends Phase 2 work using the existing bounded session behavior; explicit
cancellation and graceful shutdown are completed in Phase 7.

## GUI State

The GUI state models receiver selection, connection lifecycle, current partial
main-zone state, field errors, freshness, authority, and the last actionable
operation error. Views derive enabled actions exclusively from that state.
Connection and availability must be conveyed with text or accessible labels in
addition to styling.

The initial flow is:

1. Load or migrate configuration.
2. Auto-connect when a saved receiver exists; otherwise show receiver setup.
3. Publish connection state and an authoritative five-field refresh.
4. Continue receiving typed events until disconnect, reselection, or exit.
5. Invalidate state and refresh after every successful reconnect.

## Configuration Migration

A platform-directory resolver supplies the shared CLI and GUI configuration
path. Loading follows a fixed precedence:

1. Read the platform file when it exists.
2. Otherwise parse the legacy relative YAML file.
3. If valid, save the same identity to the platform path and continue.
4. If absent, begin unconfigured; if invalid or unwritable, report the error
   without deleting or replacing the legacy file.

Configuration continues to contain receiver identity only. No credentials or
GUI window state are added to the v1 shape.

## Verification Boundaries

Reducer and update-loop tests cover every visible lifecycle state. Injected
configuration paths cover precedence, import, invalid legacy data, and failed
writes. Local TCP servers cover initial refresh, unsolicited events,
disconnect, reconnect invalidation, and fresh authority. UI tests verify that
recovery actions remain reachable by keyboard and that unavailable state is
not communicated by color alone.
