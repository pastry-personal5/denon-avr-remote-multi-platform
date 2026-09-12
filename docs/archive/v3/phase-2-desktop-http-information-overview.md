# Version 3, Phase 2 — Desktop HTTP receiver information

## Status

**Implemented — 2026-09-09.** The workspace package version remains 2.0.0.

For a selected AVR-X3800H or AVC-X3800H, Dashboard shows typed input and
speaker layouts plus Audio, Video, and Audyssey receiver summaries. This is a
read-only display feature. Other models show unavailable values and do not
make these HTTP requests.

The dashboard refreshes the information after authoritative Main Zone status,
reconnect, input/surround changes, and power-on, then every 30 seconds while
connected with Main Zone on. Disconnect, standby, and context changes clear
the display. HTTP errors retain a same-connection last observation when one
exists; they never invalidate or delay normal Main Zone status.

No endpoint names, XML, addresses, raw control values, or request evidence is
presented in the desktop UI.
