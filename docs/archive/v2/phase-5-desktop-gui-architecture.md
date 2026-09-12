# Version 2 - Phase 5 Architecture

```text
Iced GUI presentation
        |
  controller bridge / subscription
        |
  ReceiverController
    /       |       \
 config  discovery  ReceiverSession
                         |
                   AVR protocol/transport
```

Phase 5 is a presentation integration layer above the Phase 4 controller.
The GUI uses `ControllerHandle` and typed `ReceiverCommand` values. It does
not construct AVR strings, open sockets, select retry behavior, confirm
controls, or persist live receiver state.

## Iced Runtime Contract

Use `iced::application(boot, update, view)` with a stable subscription keyed
to controller lifetime. `boot` creates GUI state and initial tasks; `update`
maps messages to typed controller commands and bounded tasks; `view` is a
side-effect-free projection of state.

The bridge owns the controller event receiver and forwards
`ReceiverEvent` values as GUI messages. Request identities and receiver
connection generations are carried through task and event messages. Messages
from a superseded request, inactive receiver, or old generation are dropped.

## State and Lifecycle

GUI state contains route, window class, receiver selection, lifecycle,
snapshot, field errors, resource version, control operation state, setup form,
diagnostics, focus intent, and announcements. It reuses typed domain snapshots
and controller outcomes rather than creating a second business-state model.

Saved identities, discovered identities, and explicit hosts use the existing
selection policy. A selection change invalidates visible authority. Connect,
refresh, reconnect, disconnect, and shutdown remain controller operations.

Shutdown stops new intents, sends controller shutdown, closes the bridge, and
waits for bounded termination. It never sends standby and never claims an
in-flight control was undone.

## Verification Boundaries

GUI tests use controller fakes and deterministic event sequences. They cover
boot, setup recovery, stale tasks/events, partial status, reconnect, control
outcomes, focus, resize, accessibility, and shutdown. Local TCP tests remain
the evidence for the concrete session and protocol adapters. Manual checks
cover Windows, macOS, and Linux behavior; packaged release artifacts are not
required.
