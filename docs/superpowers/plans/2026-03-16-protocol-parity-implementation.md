# Protocol Parity Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Achieve full MCP protocol parity with claudecode.nvim — every tool returns the exact response format Claude Code CLI expects.

**Architecture:** Fix response formats in server.rs tool handlers, add missing state fields through the full pipeline (claude.kak → CLI args → socket message → EditorState), implement full openFile with text search in Rust, add pong timeout tracking in websocket.rs.

**Tech Stack:** Rust, mio, tungstenite, serde_json, Kakoune script

**Spec:** `docs/superpowers/specs/2026-03-16-protocol-parity-design.md`

---

## Chunk 1: State Pipeline Enhancement (line_count, modified, hooks)

### Task 1: Add line_count and modified to state pipeline

**Files:**
- Modify: `src/main.rs:48-77` (SendMessage::State)
- Modify: `src/client.rs:5-18` (build_state_message)
- Modify: `src/kakoune/socket.rs:1-11` (KakMessage::State, RawMessage)
- Modify: `src/kakoune/state.rs:65-94` (EditorState)
- Modify: `rc/claude.kak:52-68` (claude-push-state)
- Test: existing tests in each module

- [ ] **Step 1: Add CLI args to SendMessage::State in main.rs**

In `src/main.rs`, add two new fields to the `State` variant inside `SendMessage`:

```rust
/// Buffer line count
#[arg(long, default_value = "0")]
line_count: u32,
/// Buffer modified status
#[arg(long, default_value = "false")]
modified: String,
```

And in the match arm (around line 141), pass them through:

```rust
SendMessage::State { client, file, line, col, selection, sel_desc, sel_len, selection_stdin, error_count, warning_count, line_count, modified } => {
    let actual_selection = if selection_stdin {
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf).unwrap_or(0);
        buf
    } else {
        selection
    };
    client::build_state_message(&client, &file, line, col, &actual_selection, &sel_desc, sel_len, error_count, warning_count, line_count, &modified)
}
```

- [ ] **Step 2: Update build_state_message in client.rs**

In `src/client.rs`, update the function signature and JSON:

```rust
pub fn build_state_message(client: &str, file: &str, line: u32, col: u32, selection: &str, sel_desc: &str, sel_len: u32, error_count: u32, warning_count: u32, line_count: u32, modified: &str) -> String {
    serde_json::json!({
        "type": "state",
        "client": client,
        "file": file,
        "line": line,
        "col": col,
        "selection": selection,
        "sel_desc": sel_desc,
        "sel_len": sel_len,
        "error_count": error_count,
        "warning_count": warning_count,
        "line_count": line_count,
        "modified": modified == "true"
    }).to_string()
}
```

Update the test `test_build_state_message` to pass the new args.

- [ ] **Step 3: Update KakMessage::State and RawMessage in socket.rs**

In `src/kakoune/socket.rs`, add fields to both structs:

```rust
// KakMessage::State - add:
line_count: u32, modified: bool

// RawMessage - add:
#[serde(default)]
line_count: u32,
#[serde(default)]
modified: bool,

// In parse() match arm for "state", add:
line_count: raw.line_count,
modified: raw.modified,
```

- [ ] **Step 4: Update EditorState to cache line_count and is_dirty**

In `src/kakoune/state.rs`, add fields to `EditorState`:

```rust
pub struct EditorState {
    cwd: String,
    current: Selection,
    latest: Selection,
    buffers: Vec<BufferInfo>,
    error_count: u32,
    warning_count: u32,
    pub line_count: u32,   // NEW - pub for test access
    pub is_dirty: bool,    // NEW - pub for test access
}
```

Update `new()` to initialize both to `0` / `false`.

Update `update_selection()` signature to accept `line_count: u32, modified: bool`:

```rust
pub fn update_selection(&mut self, text: String, file: String, line: u32, col: u32, sel_desc: String, sel_len: u32, error_count: u32, warning_count: u32, line_count: u32, modified: bool) {
    self.error_count = error_count;
    self.warning_count = warning_count;
    self.line_count = line_count;
    self.is_dirty = modified;
    // ... rest unchanged
}
```

- [ ] **Step 4b: Fix existing tests that call update_selection with old signature**

In `src/kakoune/state.rs` tests, update these 3 tests to add the two new params (`line_count`, `modified`):

```rust
// test_update_selection (line ~255): add , 0, false after the last 0
state.update_selection("hello world".into(), "/tmp/file.rs".into(), 10, 5, "10.5,10.16".into(), 11, 0, 0, 0, false);

// test_latest_selection_preserved (line ~266): same
state.update_selection("selected text".into(), "/tmp/a.rs".into(), 5, 1, "5.1,5.13".into(), 13, 0, 0, 0, false);

// test_selection_to_mcp_json (line ~285): same
state.update_selection("hi".into(), "/tmp/f.rs".into(), 3, 7, "3.7,3.9".into(), 2, 0, 0, 0, false);
```

- [ ] **Step 5: Update server.rs to pass new fields through**

In `src/server.rs`, update the `process_kak_message` State match arm (around line 620):

```rust
KakMessage::State { client, file, line, col, selection, sel_desc, sel_len, error_count, warning_count, line_count, modified } => {
    self.kak.set_client(&client);
    if file.starts_with('*') || file.is_empty() {
        return;
    }
    self.state.update_selection(selection, file, line, col, sel_desc, sel_len, error_count, warning_count, line_count, modified);
    // ... debounce unchanged
}
```

- [ ] **Step 6: Update claude.kak hooks and state push**

In `rc/claude.kak`, update `claude-push-state` to include new args:

```kak
define-command -hidden claude-push-state %{
  nop %sh{
    printf '%s' "$kak_selection" | kak-claude send --session "$kak_session" state \
      --client "$kak_client" \
      --file "$kak_buffile" \
      --line "$kak_cursor_line" \
      --col "$kak_cursor_column" \
      --selection "" \
      --sel-desc "$kak_selection_desc" \
      --sel-len "$kak_selection_length" \
      --selection-stdin \
      --error-count "${kak_opt_lsp_diagnostic_error_count:-0}" \
      --warning-count "${kak_opt_lsp_diagnostic_warning_count:-0}" \
      --line-count "${kak_buf_line_count:-0}" \
      --modified "${kak_modified:-false}" &
  }
}
```

Add new hooks to `claude-install-hooks`:

```kak
define-command -hidden claude-install-hooks %{
  hook -group claude global NormalIdle .* %{ claude-push-state }
  hook -group claude global InsertIdle .* %{ claude-push-state }
  hook -group claude global FocusIn .* %{ claude-push-state }
  hook -group claude global WinDisplay .* %{ claude-push-state; claude-push-buffers }
  hook -group claude global BufCreate  .* %{ claude-push-buffers }
  hook -group claude global BufClose   .* %{ claude-push-buffers }
  hook -group claude global KakEnd     .* %{ claude-shutdown }
}
```

- [ ] **Step 7: Build and run tests**

Run: `cargo test`
Expected: All existing tests pass (update any that broke from signature changes)

- [ ] **Step 8: Commit**

```bash
git add src/main.rs src/client.rs src/kakoune/socket.rs src/kakoune/state.rs src/server.rs rc/claude.kak
git commit -m "feat: add line_count/modified to state pipeline, add InsertIdle/FocusIn/WinDisplay hooks"
```

---

## Chunk 2: Tool Response Format Fixes

### Task 2: Fix getCurrentSelection and getLatestSelection responses

**Files:**
- Modify: `src/kakoune/state.rs:23-56` (Selection::to_mcp_json)
- Modify: `src/server.rs:446-453` (tool handlers)

- [ ] **Step 1: Add to_mcp_json_with_success to Selection**

In `src/kakoune/state.rs`, add a new method:

```rust
/// Convert to MCP JSON with success field (for getCurrentSelection)
pub fn to_mcp_json_with_success(&self) -> serde_json::Value {
    if self.file_path.is_empty() {
        return serde_json::json!({
            "success": false,
            "message": "No active editor found"
        });
    }
    let mut json = self.to_mcp_json();
    json.as_object_mut().unwrap().insert("success".into(), serde_json::Value::Bool(true));
    json
}
```

- [ ] **Step 2: Update server.rs getCurrentSelection handler**

In `src/server.rs`, change the `getCurrentSelection` handler (around line 446):

```rust
"getCurrentSelection" => {
    let json = self.state.current_selection().to_mcp_json_with_success();
    mcp_tool_response(json)
}
```

- [ ] **Step 3: Update server.rs getLatestSelection handler**

```rust
"getLatestSelection" => {
    let sel = self.state.latest_selection();
    let json = if sel.file_path.is_empty() {
        serde_json::json!({"success": false, "message": "No selection available"})
    } else {
        sel.to_mcp_json()  // no success field, matching nvim
    };
    mcp_tool_response(json)
}
```

- [ ] **Step 4: Add tests for new selection methods**

In `src/kakoune/state.rs` tests:

```rust
#[test]
fn test_selection_with_success_field() {
    let mut state = EditorState::new("/tmp".into());
    state.update_selection("hi".into(), "/tmp/f.rs".into(), 3, 7, "3.7,3.9".into(), 2, 0, 0, 100, false);
    let json = state.current_selection().to_mcp_json_with_success();
    assert_eq!(json["success"], true);
    assert_eq!(json["filePath"], "/tmp/f.rs");
}

#[test]
fn test_empty_selection_returns_failure() {
    let state = EditorState::new("/tmp".into());
    let json = state.current_selection().to_mcp_json_with_success();
    assert_eq!(json["success"], false);
    assert!(json["message"].as_str().unwrap().contains("No active editor"));
}

#[test]
fn test_latest_selection_no_success_field() {
    let mut state = EditorState::new("/tmp".into());
    state.update_selection("hi".into(), "/tmp/f.rs".into(), 3, 7, "".into(), 2, 0, 0, 0, false);
    let json = state.latest_selection().to_mcp_json();
    assert!(json.get("success").is_none());
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/kakoune/state.rs src/server.rs
git commit -m "fix: getCurrentSelection adds success field, getLatestSelection failure case"
```

### Task 3: Fix checkDocumentDirty, saveDocument, closeAllDiffTabs responses

**Files:**
- Modify: `src/server.rs:525-539` (checkDocumentDirty, saveDocument handlers)
- Modify: `src/server.rs:537-539` (closeAllDiffTabs handler)
- Modify: `src/server.rs:643-652` (DirtyResponse processing)
- Modify: `src/kakoune/state.rs` (add has_buffer method)

- [ ] **Step 1: Add has_buffer helper to EditorState**

In `src/kakoune/state.rs`:

```rust
/// Check if a buffer path exists in the buffer list
pub fn has_buffer(&self, path: &str) -> bool {
    self.buffers.iter().any(|b| b.path == path || format!("{}/{}", self.cwd, b.path) == path)
}
```

- [ ] **Step 2: Fix checkDocumentDirty — add buffer existence check**

In `src/server.rs`, update the `checkDocumentDirty` handler:

```rust
"checkDocumentDirty" => {
    let path = args["filePath"].as_str().unwrap_or("");
    if !self.state.has_buffer(path) {
        let result = mcp_tool_response(serde_json::json!({
            "success": false,
            "message": format!("Document not open: {}", path)
        }));
        let resp = JsonRpcResponse::success(id, serde_json::json!({"content": result}));
        return Some(serde_json::to_string(&resp).unwrap());
    }
    let ws_token = self.active_ws_token.unwrap_or(Token(TOKEN_START));
    self.pending_dirty.insert(path.to_string(), (id, ws_token));
    let _ = self.kak.query_dirty(path);
    return None;
}
```

- [ ] **Step 3: Fix DirtyResponse to include filePath and isUntitled**

In `src/server.rs`, update `process_kak_message(DirtyResponse)`:

```rust
KakMessage::DirtyResponse { file, dirty } => {
    if let Some((rpc_id, ws_token)) = self.pending_dirty.remove(&file) {
        let result = mcp_tool_response(serde_json::json!({
            "success": true,
            "filePath": file,
            "isDirty": dirty,
            "isUntitled": false
        }));
        let resp = JsonRpcResponse::success(rpc_id, serde_json::json!({"content": result}));
        let text = serde_json::to_string(&resp).unwrap();
        self.send_to_ws(ws_token, &text);
    }
}
```

- [ ] **Step 4: Fix saveDocument — add buffer check and full response**

```rust
"saveDocument" => {
    let path = args["filePath"].as_str().unwrap_or("");
    if !self.state.has_buffer(path) {
        let result = mcp_tool_response(serde_json::json!({
            "success": false,
            "message": format!("Document not open: {}", path)
        }));
        let resp = JsonRpcResponse::success(id, serde_json::json!({"content": result}));
        return Some(serde_json::to_string(&resp).unwrap());
    }
    let _ = self.kak.save_buffer(path);
    mcp_tool_response(serde_json::json!({
        "success": true,
        "filePath": path,
        "saved": true,
        "message": "Document saved successfully"
    }))
}
```

- [ ] **Step 5: Fix closeAllDiffTabs — return CLOSED_N_DIFF_TABS**

```rust
"closeAllDiffTabs" => {
    let count = self.state.count_diff_buffers();
    let _ = self.kak.close_diff_buffers();
    mcp_tool_response(serde_json::json!(format!("CLOSED_{}_DIFF_TABS", count)))
}
```

Add to `EditorState`:

```rust
pub fn count_diff_buffers(&self) -> usize {
    self.buffers.iter().filter(|b| b.path.contains("claude-diff")).count()
}
```

- [ ] **Step 6: Add tests for has_buffer and count_diff_buffers**

In `src/kakoune/state.rs` tests:

```rust
#[test]
fn test_has_buffer_absolute_path() {
    let mut state = EditorState::new("/home/user/project".into());
    state.update_buffers("'src/main.rs' 'src/lib.rs'");
    assert!(state.has_buffer("/home/user/project/src/main.rs"));
    assert!(state.has_buffer("src/main.rs"));
    assert!(!state.has_buffer("/other/path.rs"));
}

#[test]
fn test_count_diff_buffers() {
    let mut state = EditorState::new("/tmp".into());
    state.update_buffers("'src/main.rs' '*claude-diff*' '*claude-diff-2*'");
    assert_eq!(state.count_diff_buffers(), 2);
}
```

- [ ] **Step 7: Run tests**

Run: `cargo test`
Expected: PASS

- [ ] **Step 8: Commit**

```bash
git add src/server.rs src/kakoune/state.rs
git commit -m "fix: checkDocumentDirty/saveDocument/closeAllDiffTabs response formats"
```

### Task 4: Fix getOpenEditors response

**Files:**
- Modify: `src/kakoune/state.rs:147-176` (open_editors_json)

- [ ] **Step 1: Rewrite open_editors_json**

Replace the `open_editors_json` method in `src/kakoune/state.rs`:

```rust
pub fn open_editors_json(&self) -> serde_json::Value {
    let tabs: Vec<serde_json::Value> = self.buffers.iter()
        .filter(|b| !b.path.starts_with('*') && !b.path.starts_with("debug"))
        .map(|b| {
        let full_path = if b.path.starts_with('/') {
            b.path.clone()
        } else {
            format!("{}/{}", self.cwd, b.path)
        };
        let file_name = std::path::Path::new(&full_path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| b.path.clone());
        let lang = guess_language(&b.path);
        let is_active = b.is_active;

        let mut tab = serde_json::json!({
            "uri": format!("file://{}", full_path),
            "isActive": is_active,
            "isPinned": false,
            "isPreview": false,
            "isDirty": if is_active { self.is_dirty } else { false },
            "label": file_name,
            "groupIndex": 0,
            "viewColumn": 1,
            "isGroupActive": true,
            "fileName": full_path,
            "languageId": lang,
            "lineCount": if is_active { self.line_count } else { 0 },
            "isUntitled": false
        });

        // Add selection for active buffer
        if is_active && !self.current.file_path.is_empty() {
            let sel = &self.current;
            let line_0 = if sel.line > 0 { sel.line - 1 } else { 0 };
            let col_0 = if sel.col > 0 { sel.col - 1 } else { 0 };
            tab.as_object_mut().unwrap().insert("selection".into(), serde_json::json!({
                "start": { "line": line_0, "character": col_0 },
                "end": { "line": line_0, "character": col_0 }
            }));
        }

        tab
    }).collect();
    serde_json::json!({ "tabs": tabs })
}
```

- [ ] **Step 2: Update test_open_editors_json**

```rust
#[test]
fn test_open_editors_json() {
    let mut state = EditorState::new("/tmp".into());
    state.update_buffers("'src/main.rs' 'src/lib.rs'");
    state.line_count = 100;
    state.is_dirty = true;
    let json = state.open_editors_json();
    let tabs = json["tabs"].as_array().unwrap();
    assert_eq!(tabs.len(), 2);
    assert!(tabs[0]["uri"].as_str().unwrap().contains("main.rs"));
    assert_eq!(tabs[0]["fileName"], "/tmp/src/main.rs");
    assert_eq!(tabs[0]["isPinned"], false);
    assert_eq!(tabs[0]["isUntitled"], false);
    assert_eq!(tabs[0]["lineCount"], 100);
    assert_eq!(tabs[0]["isDirty"], true);
    assert!(tabs[0].get("diagnosticCounts").is_none());
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/kakoune/state.rs
git commit -m "fix: getOpenEditors matches nvim format with all fields"
```

---

## Chunk 3: getDiagnostics Format Fix + Ping/Pong Timeout

### Task 5: Fix getDiagnostics response format

**Files:**
- Modify: `src/server.rs:654-664` (DiagnosticsResponse processing)

- [ ] **Step 1: Rewrite DiagnosticsResponse handler**

In `src/server.rs`, replace the `DiagnosticsResponse` match arm:

```rust
KakMessage::DiagnosticsResponse { file, data } => {
    if let Some((rpc_id, ws_token)) = self.pending_diagnostics.remove(&file) {
        let diagnostics: Vec<serde_json::Value> = serde_json::from_str(&data).unwrap_or_default();
        // One content item per diagnostic (matching nvim)
        let content: Vec<serde_json::Value> = diagnostics.iter().map(|d| {
            let diag = serde_json::json!({
                "filePath": file,
                "line": d["range"]["start"]["line"].as_i64().unwrap_or(0) + 1,
                "character": d["range"]["start"]["character"].as_i64().unwrap_or(0) + 1,
                "severity": d["severity"].as_i64().unwrap_or(1),
                "message": d["message"].as_str().unwrap_or(""),
                "source": "lsp"
            });
            serde_json::json!({
                "type": "text",
                "text": serde_json::to_string(&diag).unwrap()
            })
        }).collect();

        let content = if content.is_empty() {
            serde_json::json!([{"type": "text", "text": "[]"}])
        } else {
            serde_json::Value::Array(content)
        };

        let resp = JsonRpcResponse::success(rpc_id, serde_json::json!({"content": content}));
        let text = serde_json::to_string(&resp).unwrap();
        self.send_to_ws(ws_token, &text);
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add src/server.rs
git commit -m "fix: getDiagnostics returns one content item per diagnostic"
```

### Task 6: Add ping/pong timeout tracking

**Files:**
- Modify: `src/websocket.rs:44-57` (WsConnection struct, new, read_message)
- Modify: `src/server.rs:717-726` (send_pings)

- [ ] **Step 1: Add last_pong to WsConnection**

In `src/websocket.rs`, add `last_pong` field and update methods:

```rust
pub struct WsConnection {
    state: WsState,
    write_queue: std::collections::VecDeque<Message>,
    pub authenticated: bool,
    last_pong: std::time::Instant,
}

impl WsConnection {
    pub fn new(stream: TcpStream) -> Self {
        Self {
            state: WsState::Pending(stream),
            write_queue: std::collections::VecDeque::new(),
            authenticated: false,
            last_pong: std::time::Instant::now(),
        }
    }
```

- [ ] **Step 2: Update read_message to track Pong**

In `src/websocket.rs`, change the Pong handler (line 117):

```rust
Ok(Message::Pong(_)) => {
    self.last_pong = std::time::Instant::now();
    Ok(None)
}
```

- [ ] **Step 3: Add is_alive and reset_pong_timer methods**

```rust
pub fn is_alive(&self, timeout: std::time::Duration) -> bool {
    self.last_pong.elapsed() < timeout
}

pub fn reset_pong_timer(&mut self) {
    self.last_pong = std::time::Instant::now();
}
```

- [ ] **Step 4: Update send_pings in server.rs**

In `src/server.rs`, replace `send_pings`:

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

- [ ] **Step 5: Build and run tests**

Run: `cargo test`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/websocket.rs src/server.rs
git commit -m "feat: add ping/pong timeout tracking with sleep detection"
```

---

## Chunk 4: openFile Full Implementation

### Task 7: Implement openFile with all parameters

**Files:**
- Modify: `src/kakoune/session.rs:66-81` (open_file methods)
- Modify: `src/server.rs:462-471` (openFile handler)

- [ ] **Step 1: Add new session methods**

In `src/kakoune/session.rs`, add after `open_file_at`:

```rust
/// Open a file and select a precise range using Kakoune's select command
/// All positions are 1-based (Kakoune native)
pub fn open_file_select_range(&self, path: &str, start_line: u32, start_col: u32, end_line: u32, end_col: u32) -> std::io::Result<()> {
    let escaped = path.replace('\'', "''");
    self.eval(&format!(
        "edit! '{}' {}; select {}.{},{}.{}",
        escaped, start_line, start_line, start_col, end_line, end_col
    ))
}

/// Open a file, select range, then extend to end of line
pub fn open_file_select_to_eol(&self, path: &str, start_line: u32, start_col: u32, end_line: u32) -> std::io::Result<()> {
    let escaped = path.replace('\'', "''");
    self.eval(&format!(
        "edit! '{}' {}; select {}.{},{}.999999; execute-keys <a-l>",
        escaped, start_line, start_line, start_col, end_line,
    ))
}
```

- [ ] **Step 2: Add text_search helper function in server.rs**

Add at the top of `src/server.rs` (after `compute_changed_ranges`):

```rust
/// Search for plain text in a file, returns (1-based line, 1-based byte column, match length in bytes)
fn find_text_in_file(path: &str, text: &str) -> Option<(u32, u32, usize)> {
    let contents = std::fs::read_to_string(path).ok()?;
    for (idx, line) in contents.lines().enumerate() {
        if let Some(col) = line.find(text) {
            return Some((idx as u32 + 1, col as u32 + 1, text.len()));
        }
    }
    None
}

/// Search for plain text starting from a specific line (1-based, exclusive)
fn find_text_in_file_after(path: &str, text: &str, after_line: u32) -> Option<(u32, u32, usize)> {
    let contents = std::fs::read_to_string(path).ok()?;
    for (idx, line) in contents.lines().enumerate() {
        if (idx as u32 + 1) <= after_line { continue; }
        if let Some(col) = line.find(text) {
            return Some((idx as u32 + 1, col as u32 + 1, text.len()));
        }
    }
    None
}

fn count_lines_in_file(path: &str) -> u32 {
    std::fs::read_to_string(path)
        .map(|c| c.lines().count() as u32)
        .unwrap_or(0)
}
```

- [ ] **Step 3: Rewrite openFile handler in server.rs**

Replace the `openFile` match arm in `handle_tool_call`:

```rust
"openFile" => {
    let path = args["filePath"].as_str().unwrap_or("");
    let start_line = args["startLine"].as_u64().map(|n| n as u32);
    let end_line = args["endLine"].as_u64().map(|n| n as u32);
    let start_text = args["startText"].as_str().filter(|s| !s.is_empty());
    let end_text = args["endText"].as_str().filter(|s| !s.is_empty());
    let select_to_eol = args["selectToEndOfLine"].as_bool().unwrap_or(false);
    let make_frontmost = args["makeFrontmost"].as_bool().unwrap_or(true);
    // preview is accepted but treated as normal open (Kakoune has no preview mode)
    let _preview = args["preview"].as_bool().unwrap_or(false);

    let message;

    if let Some(st) = start_text {
        // Text-based search
        if let Some((sl, sc, slen)) = find_text_in_file(path, st) {
            if let Some(et) = end_text {
                if let Some((el, ec, elen)) = find_text_in_file_after(path, et, sl) {
                    if select_to_eol {
                        let _ = self.kak.open_file_select_to_eol(path, sl, sc, el);
                    } else {
                        let _ = self.kak.open_file_select_range(path, sl, sc, el, ec + elen as u32 - 1);
                    }
                    message = format!("Opened file and selected text from \"{}\" to \"{}\"", st, et);
                } else {
                    // endText not found, select only startText
                    let _ = self.kak.open_file_select_range(path, sl, sc, sl, sc + slen as u32 - 1);
                    message = format!("Opened file and selected text \"{}\" (endText \"{}\" not found)", st, et);
                }
            } else {
                let _ = self.kak.open_file_select_range(path, sl, sc, sl, sc + slen as u32 - 1);
                message = format!("Opened file and selected text \"{}\"", st);
            }
        } else {
            let _ = self.kak.open_file(path);
            message = format!("Opened file, but text \"{}\" not found", st);
        }
    } else if let Some(sl) = start_line {
        if let Some(el) = end_line {
            if select_to_eol {
                let _ = self.kak.open_file_select_to_eol(path, sl, 1, el);
            } else {
                let _ = self.kak.open_file_select_range(path, sl, 1, el, 999999);
            }
            message = format!("Opened file and selected lines {} to {}", sl, el);
        } else {
            let _ = self.kak.open_file_at(path, sl, None);
            message = format!("Opened file at line {}", sl);
        }
    } else {
        let _ = self.kak.open_file(path);
        message = format!("Opened file: {}", path);
    }

    if !make_frontmost {
        let line_count = count_lines_in_file(path);
        let lang = crate::kakoune::state::guess_language(path);
        mcp_tool_response(serde_json::json!({
            "success": true,
            "filePath": path,
            "languageId": lang,
            "lineCount": line_count
        }))
    } else {
        mcp_tool_response(serde_json::json!(message))
    }
}
```

- [ ] **Step 4: Make guess_language public in state.rs**

In `src/kakoune/state.rs`, change `fn guess_language` to `pub fn guess_language`.

- [ ] **Step 5: Add tests for text search**

In `src/server.rs` tests:

```rust
#[test]
fn test_find_text_in_file() {
    let dir = tempfile::TempDir::new().unwrap();
    let f = dir.path().join("test.rs");
    std::fs::write(&f, "fn main() {\n    println!(\"hello\");\n}\n").unwrap();
    let result = find_text_in_file(f.to_str().unwrap(), "println");
    assert_eq!(result, Some((2, 5, 7)));
}

#[test]
fn test_find_text_in_file_not_found() {
    let dir = tempfile::TempDir::new().unwrap();
    let f = dir.path().join("test.rs");
    std::fs::write(&f, "fn main() {}\n").unwrap();
    assert!(find_text_in_file(f.to_str().unwrap(), "nonexistent").is_none());
}

#[test]
fn test_find_text_in_file_after() {
    let dir = tempfile::TempDir::new().unwrap();
    let f = dir.path().join("test.rs");
    std::fs::write(&f, "fn main() {\n    let x = 1;\n    let y = 2;\n}\n").unwrap();
    let result = find_text_in_file_after(f.to_str().unwrap(), "}", 1);
    assert_eq!(result, Some((4, 1, 1)));
}
```

- [ ] **Step 6: Run tests**

Run: `cargo test`
Expected: PASS

- [ ] **Step 7: Build full binary and verify**

Run: `cargo build`
Expected: Compiles with no errors

- [ ] **Step 8: Commit**

```bash
git add src/server.rs src/kakoune/session.rs src/kakoune/state.rs
git commit -m "feat: openFile full implementation with text search, selectToEndOfLine, makeFrontmost"
```

---

## Chunk 5: Final Verification

### Task 8: Full build and test verification

**Files:** All modified files

- [ ] **Step 1: Run full test suite**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 2: Build release binary**

Run: `cargo build --release`
Expected: Clean build

- [ ] **Step 3: Verify init output includes new hooks**

Run: `cargo run -- init 2>/dev/null | grep -c 'hook -group claude'`
Expected: `7` (NormalIdle, InsertIdle, FocusIn, WinDisplay, BufCreate, BufClose, KakEnd)

- [ ] **Step 4: Verify tool list unchanged**

Run: `cargo run -- init 2>/dev/null | head -3`
Expected: Shows the kak script header

- [ ] **Step 5: Final commit if any fixups needed**

```bash
git add -A
git commit -m "chore: final protocol parity fixups"
```
