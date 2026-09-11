# Desktop client

Start the native desktop client from the repository root:

```text
make run-gui
```

## Logs

The desktop client writes structured lifecycle, diagnostic, and visible
feedback messages to a file. It rotates the file daily, retains at most 14
daily files, removes files dated three days ago or earlier, and keeps their
combined size at or below 3 MiB. When the quota is reached, older rotated files
are removed first; extra entries are skipped if the current day's file alone
fills the quota. Iced framework warnings and errors are muted by default. Set
`RUST_LOG` (for example, `RUST_LOG=debug`) before launch to include more detail.

The log directory is platform-specific:

- macOS: `~/Library/Logs/Denon AVR Remote`
- Windows: `%LOCALAPPDATA%\\Denon AVR Remote\\logs`
- Linux and other Unix systems: `$XDG_STATE_HOME/denon-avr-remote/logs`, or
  `~/.local/state/denon-avr-remote/logs` when `XDG_STATE_HOME` is unset

Rotated files are named `denon-avr-remote.log.YYYY-MM-DD`. Logs remain local;
the app does not upload them.
