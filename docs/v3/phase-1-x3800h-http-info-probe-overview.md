# Version 3, Phase 1 — X3800H HTTP information probe reliability

## Status

**Implemented — 2026-09-09. No Version 3 release is created by this phase.**

This phase makes `x3800h-http-info-probe` a dependable, read-only evidence
collector for AVR-X3800H AppCommand observations. The workspace package version
remains **2.0.0**. It neither enables receiver features nor changes receiver
state.

## Scope

The probe retains its command line and default port:

```text
x3800h-http-info-probe HOST TIMEOUT-MS MODEL FIRMWARE [PORT]
```

`PORT` defaults to `8080`. `TIMEOUT-MS` must be positive and `PORT`
must not be zero. `MODEL` and `FIRMWARE` must not contain line or other
control characters, because they are printed in line-oriented evidence records.

It sends one primary batch to `/goform/AppCommand0300.xml`, containing only
the five fixed read-only operations: `GetInputSignal`, `GetActiveSpeaker`,
`GetVideoInfo`, `GetAudioInfo`, and `GetAudyssyInfo`. No `Set*`,
control, zone, or configuration operation is in scope.

The validated AVC-X3800H firmware is sensitive to XML layout: semantically
identical compact AppCommand XML returns an empty `<rx/>`, while
line-delimited XML returns the requested records. The protocol encoder
therefore emits a stable line-delimited wire format, including a complete
opening-and-closing `param` element on one line. This is a firmware-specific
transport constraint, not a general XML rule.

A batch is usable only when HTTP succeeds, its body parses as AppCommand XML,
and at least one of those requested command names is returned. A valid partial
batch is retained and only its missing requested commands are retried
individually at the standard `0300` endpoint. A valid empty `<rx/>` also
gets a single `0301` *endpoint-compatibility observation* before individual
`0300` retries. The `0301` record is evidence about endpoint behavior, not
support for a product capability.

Every attempt prints its endpoint, request scope, request XML, HTTP status (if
available), one-line raw XML, and every returned parameter. Parameter records
preserve the name, all attributes—including unknown attributes and control
values—and the exact returned value. HTTP, transport, and parsing failures are
separate evidence records.

The process exits with status 0 when any attempt returns a usable requested
command. It exits nonzero when every attempt is empty, malformed, or fails.
Argument misuse exits with status 2 before a connection is attempted.

## Manual validation checklist

Run:

```text
make run-diagnostics-http ARGS="HOST TIMEOUT-MS AVR-X3800H FIRMWARE"
```

Before retaining a capture, redact the host/IP address and any identifying
receiver metadata. Record model, firmware, local date/time and timezone,
endpoint, input and content, receiver UI Audio information, exact probe
output, and whether each command was batch-only, missing-and-retried, empty,
or failed. Capture a normal complete batch and any empty or partial behavior.
Confirm every request is a `Get*` operation and that no receiver setting
changes. Treat the result as firmware-specific diagnostic evidence only.
