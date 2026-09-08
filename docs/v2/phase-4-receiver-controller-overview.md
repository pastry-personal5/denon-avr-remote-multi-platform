# Version 2 - Phase 4 Overview

Phase 4 completes the application-owned `ReceiverController`. It owns
receiver selection, session lifecycle, refresh and reconnect state, Main Zone
control admission, execute-once safety, cancellation boundaries, shutdown,
timing, and redacted diagnostics. Main Zone remains the only supported zone.

Both persistent callers and the one-shot CLI use the same typed policy. A
controller does not create a Tokio runtime; callers provide the runtime.
Live values are never written to configuration—only receiver identity and
selection are persisted.

The controller accepts typed selection, connect, disconnect, refresh, control,
and shutdown commands and publishes lifecycle, partial snapshot, field error,
resource-version, control-outcome, cancellation, and diagnostic events.
Sessions expose typed queries, execute-once dispatch, receiver events, and
bounded close. AVR protocol strings remain infrastructure concerns.

Fakes cover command admission, partial snapshots, version advancement,
selection invalidation, no-op and conflict handling, cancellation, shutdown,
and bounded close. Local TCP tests continue to cover framing and the concrete
session. A live smoke test remains a release-gate activity rather than a unit
test.
