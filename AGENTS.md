# Contributor Guide

## Project shape

- `src/lib.rs` exposes the reusable library API.
- `src/avr.rs` and `src/heos.rs` contain protocol framing and parsing.
- `src/capabilities.rs` contains model and capability declarations.
- `src/main.rs` provides the command-line interface.
- `ARCHITECTURE.md` documents the intended layered design.

## Development

Use the Makefile targets where possible:

```text
make check
make test
make run ARGS="help"
make run-release ARGS="help"
```

Keep protocol code transport-independent, preserve explicit wire framing, and add tests for protocol behavior. Do not claim receiver capabilities without protocol evidence or live validation.

