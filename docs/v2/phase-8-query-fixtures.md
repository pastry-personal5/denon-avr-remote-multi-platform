# Phase 8 query fixtures

These deterministic fixtures define the typed framing used by the Phase 8
adapter. They are parser fixtures, not live capability claims; enabling a
receiver model still requires the validation record described below.

| Operation | Request | Representative response | Meaning |
| --- | --- | --- | --- |
| Quick Select recall | `MSQUICK1\r` | `MSQUICK1\r` | One Main Zone preset operation dispatched |
| MultEQ | `PSMULTEQ:?\r` | `PSMULTEQ:ON\r` | MultEQ is on |
| Dynamic EQ | `PSDYNEQ?\r` | `PSDYNEQ:OFF\r` | Dynamic EQ is off |
| Reference level | `PSREFLEV?\r` | `PSREFLEV:10\r` | A configured offset, not a boolean |
| Dynamic Volume | `PSDYNVOL?\r` | `PSDYNVOL:N/A\r` | Not applicable to the current setup |
| Audyssey LFC | `PSLFC?\r` | `PSLFC:UNAVAILABLE\r` | Unavailable, not off |
| Dirac Live | `PSDIRAC?\r` | `PSDIRAC:UNKNOWN\r` | Unknown receiver response remains unknown |

The command and response helpers preserve the Main Zone boundary. There is no
validated per-slot Quick Select query in this adapter, so registered fields
remain unknown until a future protocol investigation establishes one. A
timeout or reconnect never replays `MSQUICKn`; recall is execute-once. EQ
queries may resolve independently and retain an unavailable state for only the
failed feature.

## Live validation record

Before shipping a capability claim, record the model, firmware, receiver
settings, date, command, raw response, timing, and the resulting typed value
for every Quick Select and EQ operation. Unknown models do not inherit these
records.
