# Version 3, Phase 3 architecture — clean ports and adapters

## Package graph

```text
domain  <- application <- infrastructure <- desktop composition
   ^            ^              ^
   |            |              +-- protocol
   +------------+------------------ protocol

application + domain <- gui-lib <- desktop composition
application + domain + infrastructure <- CLI
domain + protocol + infrastructure <- read-only diagnostics
```

`domain` contains receiver vocabulary and state only. `protocol` translates
wire frames without sockets or runtime concerns. `application` owns use-case
policy and the port definitions. `infrastructure` implements those ports with
TCP AVR, HEOS, HTTP AppCommand, YAML, and SSDP adapters. GUI, CLI, desktop,
and diagnostics are delivery packages.

## Session boundary

`application::ports` owns `SessionFactory`, `ReceiverSession`, and
`SessionEvent`. A session combines asynchronous status reads, one-shot
controls, unsolicited events, optional validated read capabilities, and
shutdown because splitting ownership would permit control/event reordering.
The serialized application coordinator owns exactly one session at a time.

Synchronous and asynchronous gateway ports remain separate for the CLI and
desktop execution models. State-changing gateways promise execute-once
dispatch. The coordinator handles connection generation, stale event
rejection, refresh/invalidation, and bounded close; feature policies retain
model-validation gates and partial observations.

## Presentation and composition

The GUI library owns the serialized controller bridge and Iced subscription;
its bridge, messages, dashboard route and controls, receiver setup,
settings/diagnostics views, feedback, capture, components, and design
primitives have focused modules. It contains no protocol parser or concrete
adapter construction. The desktop executable constructs concrete adapters.

The CLI uses application contracts. Diagnostics do not share either delivery
path and issue only documented read-only probes.

## Enforcement

`tools/check-boundaries.sh` checks the allowed source imports and normal Cargo
dependency graph, rejects a reintroduced legacy tree, and rejects duplicate
session contracts. It is part of `make check`.
