# Denon AVR Remote

Rust CLI and desktop client for local Denon and Marantz AVR control. Main Zone
behavior is validated on the AVC-X3800H; unvalidated models remain read-only.

```text
make run ARGS="get status --host <receiver-ip>"
make run-gui
make check
```

The CLI discovers receivers, reads Main Zone status, and offers capability- and
evidence-gated controls. See the [documentation map](docs/README.md) for user
guides, architecture, contribution rules, commands, research evidence, active
work, and historical records.
