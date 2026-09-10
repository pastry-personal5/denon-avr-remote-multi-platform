# Version 2 - Phase 7 Overview

**Status: Complete — 2026-09-08**

Phase 7 adds read-only audio-signal and channel-context diagnostics for the
validated AVR-X3800H. It provides the diagnostic probes and
evidence-preserving observations needed to correlate receiver responses with
the receiver's `INFO` → `Audio` display and front-panel channel indicators,
without promoting model/firmware-specific behavior into the supported API.
Live TCP and HTTP traces have been collected from an AVR-X3800H, but
receiver-specific field meaning is not promoted without scenario-based
validation. The [Phase 7 technical record](phase-7-audio-context.md) describes
the protocol, adapter, and observation model in detail.

## Objective

Observe the receiver's live audio context — selected input, input mode,
digital mode, current sound mode, channel trims, and candidate codec,
channel, and sample-rate fields — through two read-only paths:

- the documented AVR Telnet queries plus the extended diagnostic candidate
  families, and
- the read-only `AppCommand0300.xml` HTTP endpoint.

The observations feed the persistent session snapshot so that capability
gates can distinguish validated, observed, and unknown context. Group recall
remains distinct from individual mode selection, and individual listening-mode
choices stay disabled without validated signal and speaker context.

## Scope

Phase 7 delivers:

- `x3800h-context-probe`, a Telnet diagnostic binary that issues the baseline
  queries `SI?`, `SD?`, `DC?`, `MS?`, and `CV?` plus the candidate queries
  `SYSDA ?`, `OPINFINS ?`, `OPINFASP ?`, `SYSMI ?`, and `SSINFAISFSV ?`.
- `x3800h-http-info-probe`, an HTTP diagnostic binary that posts the
  read-only `GetAudioInfo`, `GetInputSignal`, and `GetActiveSpeaker`
  operations.
- A typed, read-only AppCommand XML protocol
  (`crates/protocol/src/app_command.rs`) and a bounded synchronous HTTP adapter
  (`crates/infrastructure/src/app_command_http.rs`).
- Audio-context query primitives and an `AudioContextSnapshot` that feed the
  session snapshot (`crates/domain/src/audio_context.rs`, application ports).
- Parser/framing fixtures and a validation-record template for live X3800H
  correlation runs.

Out of scope:

- Promoting AppCommand or candidate Telnet field meaning to supported
  capabilities; they remain model/firmware-scoped diagnostic evidence.
- State-changing AppCommand operations; the typed queries reject them at
  construction time.
- Inferring input or output channel maps from `CV?`, `MS?`, or any single
  response.
- The browser web-control Information page as a supported API.
- GUI changes, additional zones, and other receiver models.

## Key Deliverables

- `tools/x3800h-context-probe.rs`: Telnet probe with response-family
  correlation, unsolicited-line retention, and reconnect after a failed
  query.
- `tools/x3800h-http-info-probe.rs`: HTTP probe with raw XML and
  per-parameter `name`/`control`/value output.
- `crates/protocol/src/app_command.rs`: request construction from typed read-only
  query objects and a lossless response parser.
- `crates/infrastructure/src/app_command_http.rs`: bounded HTTP client with one
  connection per exchange and explicit framing and size limits.
- `crates/domain/src/audio_context.rs`: `AudioContextSnapshot`, `Observed<T>`,
  `Observation<T>`, `Provenance`, `Confidence`, and `RawObservation`.
- `query_audio_context` on the `ReceiverSession` boundary, implemented by the
  synchronous TCP adapter and the persistent AVR session, and exposed through
  the `query_audio_context_async` use case.
- Parser/framing fixtures and the live-validation record template.

## Acceptance Criteria

- Every observation is represented as `Known`, `Unavailable`, or
  `NotValidated`, with the raw response and provenance where applicable.
- Raw evidence is preserved verbatim — padded text, unknown attributes, and
  numeric control codes — and is never silently normalized.
- Unsupported, malformed, and timed-out responses keep an independent
  unavailable status; one failure cannot erase the other observations.
- Unsolicited and event lines are retained, and a delayed response is never
  attributed to a later query.
- AppCommand queries are typed read-only; operation names that do not start
  with `Get` are rejected before any request is built.
- Every HTTP exchange is bounded in size and time, and malformed roots,
  duplicate headers, truncated bodies, and invalid status lines are rejected
  explicitly.
- Individual listening-mode choices remain disabled without validated signal
  and speaker context; group recall (`MSMOVIE`, `MSMUSIC`, `MSGAME`) remains
  a distinct operation.
- Live traces record model, firmware, input, content, settings, and date, and
  redact network identity before being committed.
