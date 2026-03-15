# kak-claude: Claude Code IDE Integration for Kakoune

## Overview

A standalone Rust daemon that bridges Kakoune and Claude Code CLI via the same WebSocket MCP protocol used by VS Code, Neovim, and Emacs integrations. Kakoune becomes a first-class Claude Code IDE.

## Architecture

Single-process daemon with a single mio event loop managing three I/O sources:

```
                    ┌─────────────────────────────┐
                    │     kak-claude daemon        │
                    │                              │
 Kakoune hooks ───► │  Unix Socket ──┐             │
 (NormalIdle etc)   │                │  mio Poll   │
                    │  TCP/WebSocket─┤  event loop  │
 Claude CLI ◄─────► │               │             │
                    │  State ────────┘             │
                    │  (selection, buffers, cwd)   │
                    └──────────┬──────────────────┘
                               │ kak -p
                               ▼
                           Kakoune
```

### mio Token Allocation

- Token 0: Unix socket listener (Kakoune state updates)
- Token 1: TCP listener (WebSocket from Claude CLI)
- Token 2+: Active connections (Unix clients, WebSocket clients)

### State Held in Memory

- Primary selection (text, file path, line/col range)
- Open buffers list (path, modified flag, language)
- Workspace folder (cwd passed at daemon start)
- Pending diff responses (deferred MCP responses keyed by request ID)

### Binary Dual Purpose

The `kak-claude` binary serves as both daemon and client:

- `kak-claude --session <s> --client <c> --cwd <dir>` — start daemon
- `kak-claude send --session <s> '<json>'` — send message to running daemon's Unix socket

Same pattern as kak-tree-sitter.

## Daemon Lifecycle

1. User runs `:claude` in Kakoune
2. Plugin spawns `kak-claude --session $kak_session --client $kak_client --cwd $(pwd)`
3. Daemon daemonizes, writes PID to `$TMPDIR/kak-claude/<session>/pid`
4. Creates Unix socket at `$TMPDIR/kak-claude/<session>/sock`
5. Starts WebSocket server on random localhost port
6. Writes lock file to `$CLAUDE_CONFIG_DIR/ide/<port>.lock` (defaults to `~/.claude/ide/<port>.lock`)
7. Plugin opens `:terminal` with `CLAUDE_CODE_SSE_PORT=<port> ENABLE_IDE_INTEGRATION=true claude`
8. On `KakEnd` hook, plugin sends shutdown via `kak-claude send`, daemon cleans up lock file + socket + PID

## MCP Protocol

### Handshake

1. Claude CLI reads lock file, gets port + auth token
2. Connects WebSocket with `x-claude-code-ide-authorization` header
3. Sends `initialize` → daemon responds with capabilities + server info
4. Sends `notifications/initialized`
5. Sends `tools/list` → daemon returns core tools
6. May send `prompts/list` → daemon responds with `{"prompts": []}`
7. Normal operation: `tools/call` requests

### Initialize Response

```json
{
  "protocolVersion": "2024-11-05",
  "capabilities": {
    "logging": {},
    "prompts": { "listChanged": true },
    "resources": { "subscribe": true, "listChanged": true },
    "tools": { "listChanged": true }
  },
  "serverInfo": {
    "name": "kak-claude",
    "version": "0.1.0"
  }
}
```

### Lock File Format

Path: `$CLAUDE_CONFIG_DIR/ide/<port>.lock` (defaults to `~/.claude/ide/<port>.lock`)

```json
{
  "pid": 12345,
  "workspaceFolders": ["/path/to/project"],
  "ideName": "Kakoune",
  "transport": "ws",
  "authToken": "<uuid-v4>"
}
```

### Message Format

JSON-RPC 2.0 wrapped in MCP envelope:

```json
{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"{...}"}]}}
```

Protocol version: `2024-11-05`

### Tool Response Format

All tool responses use the MCP content envelope. The inner `text` is always a JSON-stringified payload (double-encoded):

```json
{
  "content": [
    {
      "type": "text",
      "text": "{\"success\":true,\"data\":...}"
    }
  ]
}
```

### Core Tools (Phase 1)

| Tool | Input | Implementation |
|---|---|---|
| `getCurrentSelection` | none | Return cached state from last hook update |
| `getLatestSelection` | none | Return most recent selection even if user switched to Claude terminal |
| `getOpenEditors` | none | Return cached buffer list |
| `getWorkspaceFolders` | none | Return cwd passed at startup |
| `openFile` | see schema below | `kak -p` → `eval -client <client> 'edit <path>; select <range>'` |
| `openDiff` | `old_file_path`, `new_file_path`, `new_file_contents`, `tab_name` | Write to tmpfiles, show difft, prompt accept/reject |
| `checkDocumentDirty` | `filePath` | Non-blocking query via Unix socket round-trip |
| `saveDocument` | `filePath` | `kak -p` → `eval -client <client> 'write'` |
| `closeAllDiffTabs` | none | `kak -p` → close all `*claude-diff*` buffers |

#### `openFile` Input Schema

```json
{
  "filePath": "string (required)",
  "preview": "boolean (default false)",
  "startLine": "integer (optional)",
  "endLine": "integer (optional)",
  "startText": "string (optional, text-based selection)",
  "endText": "string (optional)",
  "selectToEndOfLine": "boolean (default false)",
  "makeFrontmost": "boolean (default true)"
}
```

All tool input schemas include `"$schema": "http://json-schema.org/draft-07/schema#"` and `"additionalProperties": false`.

#### `getCurrentSelection` / `getLatestSelection` Response

```json
{
  "text": "selected text content",
  "filePath": "/absolute/path/to/file",
  "fileUrl": "file:///absolute/path/to/file",
  "selection": {
    "start": { "line": 0, "character": 5 },
    "end": { "line": 3, "character": 10 },
    "isEmpty": false
  }
}
```

`getLatestSelection` differs in that it preserves the last visual selection even after the user switches focus to the Claude terminal. `getCurrentSelection` returns the current cursor position if no active selection.

#### `getOpenEditors` Response

```json
{
  "tabs": [
    {
      "uri": "file:///path/to/file.rs",
      "isActive": true,
      "isDirty": false,
      "label": "file.rs",
      "languageId": "rust",
      "lineCount": 150,
      "fileName": "file.rs"
    }
  ]
}
```

#### `getWorkspaceFolders` Response

```json
{
  "success": true,
  "folders": [{ "name": "project", "uri": "file:///path/to/project", "path": "/path/to/project" }],
  "rootPath": "/path/to/project"
}
```

### Selection Tracking

- Kakoune hooks push primary selection (main selection only) to daemon via Unix socket on `NormalIdle`/`InsertIdle`
- Daemon caches as both "current" and "latest" (latest preserved when user switches to Claude terminal)
- Broadcasts `selection_changed` notification to Claude CLI over WebSocket:

```json
{
  "jsonrpc": "2.0",
  "method": "selection_changed",
  "params": {
    "text": "selected text",
    "filePath": "/path/to/file",
    "fileUrl": "file:///path/to/file",
    "selection": {
      "start": { "line": 0, "character": 5 },
      "end": { "line": 3, "character": 10 },
      "isEmpty": false
    }
  }
}
```

- Debounce at 100ms
- 0-based line/character positions (LSP convention)

### WebSocket Keepalive

Daemon sends WebSocket ping frames every 30 seconds. Clients not responding with pong within 30 seconds are considered dead and disconnected.

## Diff Review Flow

When Claude calls `openDiff`:

1. Daemon receives `old_file_path`, `new_file_path`, `new_file_contents`, `tab_name`
2. Writes `new_file_contents` to a temp file
3. Sends to Kakoune via `kak -p`:
   ```
   fifo -name '*claude-diff*' -scroll -- difft --width <width> --color always <old> <new>
   ```
4. After fifo buffer opens, prompts: `Accept changes? (y/n)`
5. MCP response is deferred (stored in pending map keyed by request ID)
6. User answers:
   - `y` → daemon applies new content to file, sends MCP success response
   - `n` → daemon sends MCP rejection response
7. Kakoune picks up file change via `autoreload true`

Prompt result flows: Kakoune `prompt` → `kak-claude send` → Unix socket → daemon matches pending request → deferred WebSocket response.

## Kakoune Plugin (`rc/claude.kak`)

```kak
# Options
declare-option -hidden str claude_pid
declare-option -hidden str claude_socket
declare-option -hidden str claude_ws_port

# Main command — starts daemon + opens Claude CLI terminal
define-command claude -docstring 'Start Claude Code IDE integration' %{
  evaluate-commands %sh{
    socket="$TMPDIR/kak-claude/$kak_session/sock"
    pidfile="$TMPDIR/kak-claude/$kak_session/pid"

    # Check if daemon already running
    if [ -f "$pidfile" ] && kill -0 "$(cat "$pidfile")" 2>/dev/null; then
      port=$(cat "$TMPDIR/kak-claude/$kak_session/port")
      printf "set-option global claude_ws_port '%s'\n" "$port"
      printf "claude-open-terminal\n"
      exit
    fi

    # Start daemon — blocks until socket is ready (daemon writes port to stdout)
    port=$(kak-claude start --session "$kak_session" --client "$kak_client" --cwd "$(pwd)")

    printf "set-option global claude_socket '%s'\n" "$socket"
    printf "set-option global claude_ws_port '%s'\n" "$port"
    printf "claude-install-hooks\n"
    printf "claude-open-terminal\n"
  }
}

# Install hooks for state tracking
define-command -hidden claude-install-hooks %{
  hook -group claude global NormalIdle .* %{ claude-push-state }
  hook -group claude global InsertIdle .* %{ claude-push-state }
  hook -group claude global BufCreate  .* %{ claude-push-buffers }
  hook -group claude global BufClose   .* %{ claude-push-buffers }
  hook -group claude global KakEnd     .* %{ claude-shutdown }
}

# State push — uses kak-claude send with key=value pairs
# The send subcommand builds proper JSON internally (avoids shell escaping issues)
define-command -hidden claude-push-state %{
  nop %sh{
    kak-claude send --session "$kak_session" state \
      --file "$kak_buffile" \
      --line "$kak_cursor_line" \
      --col "$kak_cursor_column" \
      --selection "$kak_selection" &
  }
}

# Buffer list push — send subcommand splits kak_buflist correctly
define-command -hidden claude-push-buffers %{
  nop %sh{
    kak-claude send --session "$kak_session" buffers \
      --list "$kak_buflist" &
  }
}

# Shutdown daemon
define-command -hidden claude-shutdown %{
  nop %sh{
    kak-claude send --session "$kak_session" shutdown
  }
  remove-hooks global claude
}

# Open Claude CLI terminal
define-command -hidden claude-open-terminal %{
  try %{
    terminal sh -c "CLAUDE_CODE_SSE_PORT=%opt{claude_ws_port} ENABLE_IDE_INTEGRATION=true claude"
  } catch %{
    echo -markup '{Error}Run claude manually with CLAUDE_CODE_SSE_PORT=%opt{claude_ws_port} ENABLE_IDE_INTEGRATION=true'
  }
}
```

### Key design notes for the plugin

- **No shell JSON construction**: `kak-claude send` accepts typed subcommands (`state`, `buffers`, `shutdown`) with `--key value` args. The binary builds proper JSON internally, avoiding shell escaping bugs with `$kak_selection` and `$kak_buflist`.
- **Startup race condition avoided**: `kak-claude start` blocks until the daemon's Unix socket is ready, then prints the WebSocket port. Hooks are installed only after the socket exists.
- **Background sends**: State push runs `kak-claude send &` (backgrounded) to avoid blocking Kakoune's event loop.
- **Hook group**: All hooks use `-group claude` for clean removal on shutdown.

## File Structure

```
~/Sites/rust/kak-claude/
├── Cargo.toml
├── src/
│   ├── main.rs              # CLI parsing (clap), daemonize, entry
│   ├── server.rs            # mio event loop, token routing
│   ├── websocket.rs         # tungstenite over mio TcpStream, auth validation
│   ├── mcp/
│   │   ├── mod.rs
│   │   ├── protocol.rs      # JSON-RPC 2.0 types, MCP envelope
│   │   └── tools.rs         # Tool handlers
│   ├── kakoune/
│   │   ├── mod.rs
│   │   ├── socket.rs        # Unix socket listener, message parsing
│   │   ├── session.rs       # kak -p command sender
│   │   └── state.rs         # Cached state
│   ├── lockfile.rs          # Lock file management
│   └── client.rs            # kak-claude send subcommand
└── rc/
    └── claude.kak           # Kakoune plugin
```

## Dependencies

```toml
[dependencies]
mio = { version = "1", features = ["net", "os-poll", "os-ext"] }
tungstenite = "0.24"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
clap = { version = "4", features = ["derive"] }
uuid = { version = "1", features = ["v4"] }
daemonize = "0.5"
libc = "0.2"
```

No tokio, no async runtime. ~7 direct dependencies.

## Non-blocking `checkDocumentDirty`

Instead of using `kak -p` with a response fifo (which would block the mio loop), `checkDocumentDirty` uses the Unix socket round-trip:

1. Daemon sends `kak -p` command that makes Kakoune execute: `kak-claude send --session <s> dirty-response --file <path> --dirty %val{modified}`
2. Daemon registers a pending callback keyed by file path
3. When the Unix socket receives the `dirty-response`, daemon resolves the pending MCP response

Same async pattern used for diff accept/reject responses.

## Future Phases

- Phase 2: `getDiagnostics` (integrate with kakoune-lsp)
- Phase 2: Selection tracking for multiple Kakoune clients
- Phase 3: Custom MCP tools (expose Kakoune commands to Claude)
