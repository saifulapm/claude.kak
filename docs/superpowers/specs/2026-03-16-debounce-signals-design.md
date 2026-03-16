# Debounce + Signal Handling

## Overview

Two small improvements to kak-claude daemon reliability and performance.

## Debounce Selection Updates

### Problem

Every `NormalIdle` hook fires `kak-claude send state ...` which spawns a process and sends to the daemon, which broadcasts to Claude CLI over WebSocket. During rapid cursor movement this creates unnecessary load.

### Solution (both sides)

**Daemon side** (100ms debounce):
- Add `last_selection_broadcast: Instant` and `pending_selection: bool` to Server
- On receiving `State` message: always update internal state, but only broadcast if >=100ms since last broadcast
- If debounced (skipped), set `pending_selection = true`
- Adjust mio poll timeout to `min(5s, time_until_debounce_expires)` so pending state gets flushed
- After poll returns, if `pending_selection` and debounce expired, broadcast

**Plugin side** (skip InsertIdle):
- Remove `InsertIdle` from `claude-install-hooks` — cursor position during typing isn't useful to Claude
- Keep `NormalIdle` which fires after the user stops moving in normal mode

## Signal Handling

### Problem

No SIGTERM/SIGINT handler. If the daemon is killed (e.g., `kill PID`), lock files and temp files aren't cleaned up.

### Solution (self-pipe + mio)

- Create a pipe with `libc::pipe`
- Register the read end with mio Poll as a new token `SIGNAL_PIPE`
- Set a global `AtomicBool` flag from signal handlers, write a byte to the pipe to wake mio
- In the event loop, when `SIGNAL_PIPE` is readable, set `should_quit = true`
- `cleanup()` already handles all file cleanup

Signals to handle: SIGTERM, SIGINT, SIGHUP.

## Files Changed

- `src/server.rs` — debounce logic, signal pipe token, poll timeout adjustment
- `src/main.rs` — signal handler setup with self-pipe
- `rc/claude.kak` — remove InsertIdle hook
