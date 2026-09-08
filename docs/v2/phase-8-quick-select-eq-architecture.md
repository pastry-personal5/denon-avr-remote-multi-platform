# Version 2 - Phase 8 Architecture

**Status: Planned — complete before Phases 9 and 10.**

```text
Iced Quick Select / EQ views
              |
        ReceiverController
          /           \
  Quick Select       EQ status queries
        |                 |
  typed AVR adapter   typed AVR adapter
        \                 /
              AVR session
```

Phase 8 adds typed Main Zone preset and audio-processing status boundaries.
The GUI remains a presentation adapter and does not construct protocol
strings, infer processing state, or treat preset recall as a collection of
independent user commands.

## Quick Select Model

Represent four Main Zone slots with a stable slot identifier, receiver display
name, availability, and a registered-item summary. Registered items are
explicitly distinguished from omitted or unknown items. The model supports
input source, volume, sound mode, channel levels, Audyssey parameters,
Restorer, Dialog Enhancer, HDMI video output, speaker preset, Dirac Live, and
other fields only when the receiver capability record validates them.

Recall is one typed application operation. It is serialized, execute-once,
resource-version-aware where applicable, and reports authoritative or
unconfirmed completion without replaying the preset after transport failure.
Saving names, changing registered-item selection, and writing a preset are
separate typed operations and remain disabled until their X3800H wire behavior
is validated.

## EQ Status Model

Expose independent typed status for MultEQ XT32, Dynamic EQ, reference-level
offset, Dynamic Volume, Audyssey LFC, and Dirac Live. Use explicit states for
on, off, configured, unavailable, unsupported, not applicable, and unknown.
The current sound mode and calibration state may make an item unavailable;
that condition must not be reported as off.

Status queries and unsolicited updates use the existing connection-generation,
partial-field, authority, freshness, and diagnostics rules. The Dashboard
shows a compact summary; Diagnostics shows the per-feature evidence, query
time, and reason for unavailable/unknown state.

## Protocol and Evidence

The existing AVR session remains the serialized transport. Candidate Quick
Select and EQ commands must be confirmed against the AVC/AVR-X3800H rather
than inferred from older Denon protocol manuals. Older documented patterns
such as `PSMULTEQ:?`, `PSDYNEQ?`, and `PSREFLEV?` are candidate evidence only.
HTTP/web-control endpoints are not introduced as a fallback transport.

Every enabled operation requires an automated protocol fixture and a live
validation record containing model, firmware, receiver settings, command,
response, timing, and date. Unknown models never inherit X3800H capabilities.

## Verification

Tests cover slot discovery/display, preset recall, omitted registered fields,
stale versions, execute-once uncertainty, reconnect invalidation, independent
EQ states, Direct/Pure Direct restrictions, missing Audyssey calibration,
unknown responses, partial failures, Diagnostics wording, keyboard access,
and focus preservation. Live receiver validation is a release gate.
