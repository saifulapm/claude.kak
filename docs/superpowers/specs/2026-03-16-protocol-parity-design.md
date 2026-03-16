# kak-claude Protocol Parity & Kakoune-Native Improvements

## Goal

Achieve full MCP protocol parity with claudecode.nvim while leveraging Kakoune's native strengths. Every tool response must match the format Claude Code CLI expects. No gaps, no wrong formats.

## Reference

- claudecode.nvim (reference implementation): `/Users/saiful/Sites/rust/claudecode.nvim/`
- Kakoune docs: `/opt/homebrew/Cellar/kakoune/2025.06.03/share/kak/doc/`
- MCP protocol version: `2024-11-05`

---

## 1. openFile — Full Implementation

### Current State
Only handles `filePath`, `startLine`, `endLine`. Ignores `startText`, `endText`, `selectToEndOfLine`, `makeFrontmost`, `preview`. Always returns `{"success": true}`.

### Design

**session.rs changes:**

| Params | Kakoune Commands |
|--------|-----------------|
| `filePath` only | `edit! 'path'` |
| `+ startLine` (no endLine) | `edit! 'path' <startLine>` — cursor positioning only, no selection |
| `+ startLine + endLine` | `edit! 'path' <startLine>; select <startLine>.1,<endLine>.999999` |
| `+ startText` | Implemented in Rust: read file, search line-by-line for plain text match, then `edit! 'path' <line>; select <line>.<col>,<line>.<col+len>` |
| `+ startText + endText` | Search startText line-by-line. Then search endText line-by-line starting from the line *after* startText match. Select range from start of startText match to end of endText match. If endText not found, select only startText match. |
| `+ selectToEndOfLine` | Append `execute-keys <a-l>` after any selection |
| `makeFrontmost=false` | Kakoune has no background buffer concept. Open the buffer normally but return metadata response instead of message. |
| `preview` | Treated same as normal open (Kakoune has no preview mode). |

**Text search approach:** Since Kakoune's regex engine doesn't support `\Q` literal quoting, and nvim does plain-text search (not regex), we do text search in Rust:
1. Read file contents with `std::fs::read_to_string(path)`
2. Search line-by-line for `start_text` substring match (first occurrence)
3. If found, compute line number and column offset
4. Generate `select` command with exact coordinates
5. For `endText`: continue searching from line after startText match

**Response formats (matching nvim):**
- `makeFrontmost=true` (default): `{content: [{type: "text", text: "Opened file and selected lines X to Y"}]}`
- `makeFrontmost=false`: `{content: [{type: "text", text: "{\"success\":true,\"filePath\":\"/path\",\"languageId\":\"rust\",\"lineCount\":42}"}]}`

For `makeFrontmost=false`, get `lineCount` by counting lines in the file we just read (or from cached state). No need for deferred response.

**New session.rs methods:**
- `open_file_select_range(path, start_line, start_col, end_line, end_col)` — uses `select` command
- `open_file_select_to_eol(path, line)` — uses `execute-keys <a-l>`

---

## 2. Tool Response Format Fixes

### getCurrentSelection

**Current:** Returns selection JSON directly via `to_mcp_json()`.
**Fix:** Add `"success": true` to the JSON object.

```rust
// Selection::to_mcp_json() returns {text, filePath, fileUrl, selection}
// Wrap: add "success": true
```

When no active editor (empty file_path):
```json
{"success": false, "message": "No active editor found"}
```

### getLatestSelection

**Current:** Always returns selection (possibly empty).
**Fix:** When no selection has been made (empty file_path), return failure. When selection exists, return raw selection data WITHOUT `success` field (matching nvim — only getCurrentSelection adds `success: true`).

```json
// No selection: {"success": false, "message": "No selection available"}
// Has selection: {text, filePath, fileUrl, selection} — no success field
```

### checkDocumentDirty

**Current response:** `{"success": true, "isDirty": true}`
**Fixed response:** `{"success": true, "filePath": "/path", "isDirty": true, "isUntitled": false}`
**Not open:** Check `EditorState.buffers` for the path first. If not found: `{"success": false, "message": "Document not open: /path"}`

Changes in `process_kak_message(DirtyResponse)`:
- Include `filePath` from the response
- Add `isUntitled: false` (Kakoune buffers always have names)

### saveDocument

**Current response:** `{"success": true}`
**Fixed response:** `{"success": true, "filePath": "/path", "saved": true, "message": "Document saved successfully"}`
**Not open:** Check `EditorState.buffers` before sending command. If not found: `{"success": false, "message": "Document not open: /path"}`

Decision: Check `EditorState.buffers` for the path before sending `write`. This avoids needing a deferred response. If buffer exists in our list, send write command and return success with filePath.

### closeAllDiffTabs

**Current:** `{"success": true}` wrapped in MCP content
**Fixed:** Return `"CLOSED_N_DIFF_TABS"` string where N is count.

Count: count buffers matching `*claude-diff*` pattern in `EditorState.buffers` before sending delete command. If no diff buffers found, N=0.

### getOpenEditors — Missing Fields

**Add to each tab entry:**
```json
{
  "isPinned": false,
  "isPreview": false,
  "groupIndex": 0,
  "viewColumn": 1,
  "isGroupActive": true,
  "isUntitled": false,
  "lineCount": 42,
  "fileName": "/full/path/to/file.rs"
}
```

Key fixes:
- `isPinned`, `isPreview`, `groupIndex`, `viewColumn`, `isGroupActive` — hardcode sensible defaults
- `isUntitled` — false for all named buffers
- `lineCount` — cached from state push (see Section 3)
- `fileName` — use full path (not basename) to match nvim
- **Remove `diagnosticCounts`** — nvim doesn't include this
- Add `selection` field for active buffer (from cached current selection)

---

## 3. Hooks — Better State Coverage

### Current hooks (in claude.kak)
```
NormalIdle → claude-push-state
BufCreate  → claude-push-buffers
BufClose   → claude-push-buffers
KakEnd     → claude-shutdown
```

### New hooks to add

**InsertIdle** — Track cursor during insert mode (nvim tracks CursorMovedI):
```kak
hook -group claude global InsertIdle .* %{ claude-push-state }
```

**FocusIn** — Multi-window focus awareness:
```kak
hook -group claude global FocusIn .* %{ claude-push-state }
```

**WinDisplay** — Buffer switch detection (faster than waiting for NormalIdle):
```kak
hook -group claude global WinDisplay .* %{ claude-push-state; claude-push-buffers }
```

Note: Kakoune does NOT have a `BufWritePost` hook. The available write-related hook is `BufWritePre` (before write). Instead of hooking post-write, we rely on `NormalIdle` firing after the write completes (user returns to normal mode after `:w`). No post-write hook needed.

### State push enhancement

Add `$kak_buf_line_count` and `$kak_modified` to state message:

```kak
# In claude-push-state, add to kak-claude send args:
--line-count "$kak_buf_line_count" \
--modified "$kak_modified" \
```

This caches lineCount/isDirty per current buffer in EditorState for getOpenEditors.

**Changes needed:**
- `SendMessage::State` in `main.rs` — add `line_count: u32` and `modified: String` fields
- `client.rs::build_state_message()` — add new fields to JSON
- `KakMessage::State` in `socket.rs` — add new fields
- `EditorState` — store `line_count` and `is_dirty` for current buffer
- `open_editors_json()` — use cached values for active buffer

---

## 4. Ping/Pong Timeout Tracking

### Current
`send_pings()` sends WebSocket PING frames. No tracking of PONG responses.

### Design
- Add `last_pong: Instant` to `WsConnection`
- Initialize `last_pong` to `Instant::now()` on connection
- On PONG received in `read_message()`: update `last_pong`
- In `send_pings()`: close connections where `last_pong.elapsed() > 60s`
- Sleep detection: if `last_ping.elapsed() > 45s` (1.5× interval), reset all `last_pong` timestamps to now (system just woke from sleep)

**websocket.rs changes:**
```rust
pub struct WsConnection {
    // ... existing fields
    last_pong: Instant,  // NEW
}

// In read_message(), handle Pong:
Ok(Message::Pong(_)) => { self.last_pong = Instant::now(); Ok(None) }

// New method:
pub fn is_alive(&self, timeout: Duration) -> bool {
    self.last_pong.elapsed() < timeout
}
```

**server.rs changes in send_pings():**
```rust
fn send_pings(&mut self) {
    // Sleep detection: if way too long since last ping, system slept
    if self.last_ping.elapsed() >= Duration::from_secs(45) {
        for conn in self.ws_connections.values_mut() {
            conn.reset_pong_timer();
        }
    }

    let timeout = Duration::from_secs(60);
    let dead_tokens: Vec<Token> = self.ws_connections.iter_mut()
        .filter_map(|(token, conn)| {
            if !conn.is_alive(timeout) || !conn.ping() {
                Some(*token)
            } else {
                None
            }
        })
        .collect();
    for token in dead_tokens {
        self.ws_connections.remove(&token);
    }
}
```

---

## 5. getDiagnostics Format Fix

### Current
Returns single content item: `{"uri": "file:///path", "diagnostics": [...]}`

### Fixed
Return one content item per diagnostic (matching nvim):
```json
{"content": [
  {"type": "text", "text": "{\"filePath\":\"/path\",\"line\":10,\"character\":5,\"severity\":1,\"message\":\"msg\",\"source\":\"lsp\"}"},
  {"type": "text", "text": "{\"filePath\":\"/path\",\"line\":15,\"character\":1,\"severity\":2,\"message\":\"msg\",\"source\":\"lsp\"}"}
]}
```

**Changes:**
- `process_kak_message(DiagnosticsResponse)`: parse the JSON array, create one content item per diagnostic
- Each diagnostic includes: `filePath`, `line` (1-indexed), `character` (1-indexed), `severity` (integer: 1=Error, 2=Warning, 3=Information, 4=Hint), `message`, `source`
- Severity stays as integer (matching nvim which passes `diagnostic.severity` as-is)
- Line/character: the shell script already converts to 0-indexed LSP format; convert back to 1-indexed for the MCP response (`+1`)

---

## 6. RC Init from Binary

### Already Implemented
`kak-claude init` outputs `include_str!("../rc/claude.kak")` to stdout.

Users add to kakrc:
```kak
eval %sh{kak-claude init}
```

No further changes needed.

---

## 7. Terminal Launch Simplification

### Already Implemented
```kak
terminal -- env CLAUDE_CODE_SSE_PORT=%opt{claude_ws_port} ENABLE_IDE_INTEGRATION=true claude
```

No further changes needed.

---

## Files Changed

| File | Changes |
|------|---------|
| `src/server.rs` | openFile full params, response format fixes, getDiagnostics format, ping timeout, closeAllDiffTabs count, saveDocument buffer check |
| `src/kakoune/session.rs` | New open_file variants (select_range, select_to_eol) |
| `src/kakoune/state.rs` | getCurrentSelection adds success field, getLatestSelection failure case, lineCount/isDirty caching, fix open_editors_json (add missing fields, fix fileName to full path, add selection for active, remove diagnosticCounts) |
| `src/kakoune/socket.rs` | Add line_count/modified to KakMessage::State |
| `src/client.rs` | Add line_count/modified to build_state_message |
| `src/websocket.rs` | Add last_pong tracking, is_alive method, reset_pong_timer |
| `src/mcp/tools.rs` | No schema changes needed (schemas already match nvim) |
| `rc/claude.kak` | Add InsertIdle/FocusIn/WinDisplay hooks, add line-count/modified to state push |
| `src/main.rs` | Add --line-count and --modified CLI args to State subcommand |

## Unresolved Questions

None — all design decisions have been made.
