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
| `+ startLine` | `edit! 'path' <startLine>` |
| `+ startLine + endLine` | `edit! 'path' <startLine>; select <startLine>.1,<endLine>.999999` |
| `+ startText` | `edit! 'path'; execute-keys gg/\Q<text><ret>` |
| `+ startText + endText` | Search startText, then search endText forward, select range between matches |
| `+ selectToEndOfLine` | Append `execute-keys <a-l>` after any selection |
| `makeFrontmost=false` | Open buffer without switching client focus; return metadata |

**`makeFrontmost=false` response** (deferred, query Kakoune for metadata):
```json
{"success": true, "filePath": "/path", "languageId": "rust", "lineCount": 42}
```

**Implementation in server.rs `handle_tool_call`:**
- Parse all params from args
- Dispatch to appropriate `kak.open_file_*` variant
- If `makeFrontmost=false`: use deferred response pattern (query `$kak_buf_line_count`)
- If `makeFrontmost=true` (default): return simple success message string

**New session.rs methods:**
- `open_file_with_text_search(path, start_text, end_text, select_to_eol)` — uses `execute-keys /\Q<text><ret>`
- `open_file_background(path)` — opens buffer without focusing: `evaluate-commands -buffer 'path' %{}`
- `query_buffer_metadata(path)` — queries lineCount for deferred response

---

## 2. Tool Response Format Fixes

### getCurrentSelection / getLatestSelection

**Current:** Returns selection JSON directly via `to_mcp_json()`.
**Fix:** Wrap with `success` field.

```rust
// Selection::to_mcp_json() already returns {text, filePath, fileUrl, selection}
// Add "success": true to the JSON
```

For `getLatestSelection` when no selection exists (empty file_path):
```json
{"success": false, "message": "No selection available"}
```

### checkDocumentDirty

**Current response:** `{"success": true, "isDirty": true}`
**Fixed response:** `{"success": true, "filePath": "/path", "isDirty": true, "isUntitled": false}`
**Not open:** `{"success": false, "message": "Document not open: /path"}`

Changes in `process_kak_message(DirtyResponse)`:
- Include `filePath` from the response
- Add `isUntitled: false` (Kakoune buffers always have names)

### saveDocument

**Current response:** `{"success": true}`
**Fixed response:** `{"success": true, "filePath": "/path", "saved": true, "message": "Document saved successfully"}`
**Not open:** `{"success": false, "message": "Document not open: /path"}`

Make saveDocument deferred: send `kak -p` write command, get confirmation via BufWritePost or query `$kak_modified` after. Simpler alternative: assume success if `send_raw` succeeds, include filePath in response.

### closeAllDiffTabs

**Current:** `{"success": true}` wrapped in MCP content
**Fixed:** Track count of closed buffers, return `"CLOSED_N_DIFF_TABS"` string

Need to make this deferred or count locally. Simpler: track `pending_diff` count + any open diff buffers. Return `"CLOSED_{count}_DIFF_TABS"` based on pending_diff map size.

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
  "lineCount": 0
}
```

`isPinned`, `isPreview`, `groupIndex`, `viewColumn`, `isGroupActive` are VS Code concepts — hardcode sensible defaults (Kakoune has no tab groups).

`lineCount` and `isDirty`: cache per-buffer from state updates (see Section 3).

**Remove `diagnosticCounts`** — nvim doesn't include this in getOpenEditors.

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

**InsertIdle** — Track cursor during insert mode:
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

**BufWritePost** — Immediate dirty state refresh:
```kak
hook -group claude global BufWritePost .* %{ claude-push-state }
```

### State push enhancement

Add `$kak_buf_line_count` and `$kak_modified` to state message:

```kak
# In claude-push-state, add:
--line-count "$kak_buf_line_count" \
--modified "$kak_modified" \
```

This caches lineCount/isDirty per-buffer in EditorState for getOpenEditors.

**Changes needed:**
- `SendMessage::State` — add `line_count: u32` and `modified: bool` fields
- `client.rs::build_state_message()` — add new fields to JSON
- `KakMessage::State` — add new fields
- `EditorState` — store `line_count` and `is_dirty` per current buffer
- `open_editors_json()` — use cached values

---

## 4. Ping/Pong Timeout Tracking

### Current
`send_pings()` sends WebSocket PING frames. No tracking of PONG responses.

### Design
- Add `last_pong: Instant` to `WsConnection`
- On PONG received in `read_message()`: update `last_pong`
- In `send_pings()`: close connections where `last_pong.elapsed() > 60s`
- On system wake (elapsed > 45s since last ping): reset all `last_pong` timestamps (sleep detection, matching nvim's 1.5× interval grace)

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
  {"type": "text", "text": "{\"filePath\":\"/path\",\"line\":10,\"character\":5,\"severity\":\"Error\",\"message\":\"msg\",\"source\":\"lsp\"}"},
  {"type": "text", "text": "{\"filePath\":\"/path\",\"line\":15,\"character\":1,\"severity\":\"Warning\",\"message\":\"msg\",\"source\":\"lsp\"}"}
]}
```

**Changes:**
- `process_kak_message(DiagnosticsResponse)`: parse the JSON array, create one content item per diagnostic
- Each diagnostic includes: `filePath`, `line` (1-indexed), `character` (1-indexed), `severity` (string: "Error"/"Warning"/"Information"/"Hint"), `message`, `source`
- Severity mapping: 1→"Error", 2→"Warning", 3→"Information", 4→"Hint"
- Line/character: convert from 0-indexed (LSP) back to 1-indexed for the response (nvim does `+1`)

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
| `src/server.rs` | openFile full params, response format fixes, getDiagnostics format, ping timeout, closeAllDiffTabs count |
| `src/kakoune/session.rs` | New open_file variants (text search, background, metadata query) |
| `src/kakoune/state.rs` | Add success field to selection JSON, add lineCount/isDirty caching, fix open_editors_json fields, remove diagnosticCounts |
| `src/kakoune/socket.rs` | Add line_count/modified to State message |
| `src/client.rs` | Add line_count/modified to build_state_message |
| `src/websocket.rs` | Add last_pong tracking, is_alive method |
| `src/mcp/tools.rs` | No schema changes needed (schemas already match nvim) |
| `rc/claude.kak` | Add InsertIdle/FocusIn/WinDisplay/BufWritePost hooks, add line-count/modified to state push |
| `src/main.rs` | Add --line-count and --modified CLI args to State subcommand |

## Unresolved Questions

None — all design decisions have been made.
