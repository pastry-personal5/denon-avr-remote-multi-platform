# Version 3, Phase 2 — Desktop HTTP receiver information architecture

`HttpInformationSnapshot` belongs to `MainZoneSnapshot` but has independent
freshness and failure state. It holds typed channel-slot states (`Active`,
`Available`, `Absent`, or `Unknown`) and independently available receiver text
fields for Audio, Video, and Audyssey.

The protocol layer translates AppCommand response shapes into that model. The
infrastructure reader uses TCP 8080, starts with a five-command `0300` batch,
retries only missing commands individually, and attempts `0301` only after a
fully empty primary batch. All requests are fixed `Get*` queries.

The controller gates this read by the selected X3800H model capability. Its
30-second interval owns polling; the session performs bounded HTTP work off
the async task. The GUI consumes only the typed snapshot and maps slot state
to visual emphasis, so protocol metadata cannot reach the dashboard.
