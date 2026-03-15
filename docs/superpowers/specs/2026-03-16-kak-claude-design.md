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
6. Writes lock file to `~/.claude/ide/<port>.lock`
7. Plugin opens `:terminal` with `CLAUDE_CODE_SSE_PORT=<port> ENABLE_IDE_INTEGRATION=true claude`
8. On `KakEnd` hook, plugin sends shutdown via `kak-claude send`, daemon cleans up lock file + socket + PID

## MCP Protocol

### Handshake

1. Claude CLI reads `~/.claude/ide/<port>.lock`, gets port + auth token
2. Connects WebSocket with `x-claude-code-ide-authorization` header
3. Sends `initialize` → daemon responds with capabilities + tool list
4. Sends `notifications/initialized`
5. Sends `tools/list` → daemon returns core tools
6. Normal operation: `tools/call` requests

### Lock File Format

Path: `~/.claude/ide/<port>.lock`

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

### Core Tools (Phase 1)

| Tool | Input | Implementation |
|---|---|---|
| `getCurrentSelection` | none | Return cached state from last hook update |
| `getOpenEditors` | none | Return cached buffer list from last hook update |
| `getWorkspaceFolders` | none | Return cwd passed at startup |
| `openFile` | `filePath`, optional `startLine`/`endLine` | `kak -p` → `eval -client <client> 'edit <path>; select <range>'` |
| `openDiff` | `old_file_path`, `new_file_path`, `new_file_contents`, `tab_name` | Write to tmpfiles, show difft, prompt accept/reject |
| `checkDocumentDirty` | `filePath` | `kak -p` → check `%val{modified}` via response fifo |

### Selection Tracking

- Kakoune hooks push primary selection (main selection only, not all multiple selections) to daemon via Unix socket on `NormalIdle`/`InsertIdle`
- Daemon caches and broadcasts `selection_changed` notification to Claude CLI over WebSocket
- Debounce at 100ms
- 0-based line/character positions (LSP convention)

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
declare-option str claude_pid
declare-option str claude_socket
declare-option str claude_ws_port

# Main command
define-command claude %{
  # 1. Start daemon if not running (check PID file)
  # 2. Set up hooks for state tracking
  # 3. Open terminal with claude CLI + env vars
}

# Hooks (installed by :claude)
hook global NormalIdle .* %{ claude-push-state }
hook global InsertIdle .* %{ claude-push-state }
hook global BufCreate  .* %{ claude-push-buffers }
hook global BufClose   .* %{ claude-push-buffers }
hook global KakEnd     .* %{ claude-shutdown }

# State push via kak-claude send subcommand
define-command -hidden claude-push-state %{
  nop %sh{
    kak-claude send --session "$kak_session" \
      "{\"type\":\"state\",\"selection\":\"$kak_selection\",\"file\":\"$kak_buffile\",\"line\":$kak_cursor_line,\"col\":$kak_cursor_column}"
  }
}

define-command -hidden claude-push-buffers %{
  nop %sh{
    kak-claude send --session "$kak_session" \
      "{\"type\":\"buffers\",\"list\":\"$kak_buflist\"}"
  }
}

define-command -hidden claude-shutdown %{
  nop %sh{
    kak-claude send --session "$kak_session" '{"type":"shutdown"}'
  }
}

define-command -hidden claude-open-terminal %{
  try %{
    terminal sh -c "CLAUDE_CODE_SSE_PORT=$kak_opt_claude_ws_port \
      ENABLE_IDE_INTEGRATION=true claude"
  } catch %{
    echo -markup '{Error}Run claude manually with port %opt{claude_ws_port}'
  }
}
```

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

## Future Phases

- Phase 2: `getDiagnostics` (integrate with kakoune-lsp)
- Phase 2: `saveDocument` tool
- Phase 3: `closeAllDiffTabs` tool
- Phase 3: Selection tracking for multiple Kakoune clients
- Phase 3: Custom MCP tools (expose Kakoune commands to Claude)
