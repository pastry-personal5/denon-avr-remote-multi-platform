# Desktop client

Start the native desktop client from the repository root:

```text
make run-gui
```

## Logs

The desktop client writes structured lifecycle, diagnostic, and visible
feedback messages to a file. It rotates the file daily and retains the newest
14 daily files. Set `RUST_LOG` (for example, `RUST_LOG=debug`) before launch to
include more detail.

The log directory is platform-specific:

- macOS: `~/Library/Logs/Denon AVR Remote`
- Windows: `%LOCALAPPDATA%\\Denon AVR Remote\\logs`
- Linux and other Unix systems: `$XDG_STATE_HOME/denon-avr-remote/logs`, or
  `~/.local/state/denon-avr-remote/logs` when `XDG_STATE_HOME` is unset

Rotated files are named `denon-avr-remote.log.YYYY-MM-DD`. Logs remain local;
the app does not upload them.
