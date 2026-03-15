# kak-claude Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a Rust daemon that bridges Kakoune and Claude Code CLI via WebSocket MCP protocol.

**Architecture:** Single mio event loop managing a Unix socket (Kakoune state), TCP/WebSocket (Claude CLI), and cached editor state. The same binary serves as both daemon (`start`) and client (`send`).

**Tech Stack:** Rust 2021, mio 1.x, tungstenite 0.26, serde_json, clap 4, uuid, daemonize, libc

**Spec:** `docs/superpowers/specs/2026-03-16-kak-claude-design.md`

---

## Chunk 1: Project Scaffolding + Protocol Types

### Task 1: Initialize Cargo project

**Files:**
- Create: `Cargo.toml`
- Create: `src/main.rs`

- [ ] **Step 1: Create Cargo.toml**

```toml
[package]
name = "kak-claude"
version = "0.1.0"
edition = "2021"
description = "Claude Code IDE integration for Kakoune"
license = "MIT"

[dependencies]
mio = { version = "1", features = ["net", "os-poll", "os-ext"] }
tungstenite = "0.26"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
clap = { version = "4", features = ["derive"] }
uuid = { version = "1", features = ["v4"] }
daemonize = "0.5"
libc = "0.2"

[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 2: Create minimal main.rs with clap CLI**

```rust
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "kak-claude", about = "Claude Code IDE integration for Kakoune")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start the kak-claude daemon
    Start {
        /// Kakoune session name
        #[arg(long)]
        session: String,
        /// Kakoune client name
        #[arg(long)]
        client: String,
        /// Working directory
        #[arg(long)]
        cwd: String,
    },
    /// Send a message to a running daemon
    Send {
        /// Kakoune session name
        #[arg(long)]
        session: String,
        #[command(subcommand)]
        msg: SendMessage,
    },
}

#[derive(Subcommand)]
enum SendMessage {
    /// Push editor state (selection, cursor)
    State {
        #[arg(long)]
        file: String,
        #[arg(long)]
        line: u32,
        #[arg(long)]
        col: u32,
        #[arg(long)]
        selection: String,
    },
    /// Push buffer list
    Buffers {
        #[arg(long)]
        list: String,
    },
    /// Shutdown the daemon
    Shutdown,
    /// Response to dirty check
    DirtyResponse {
        #[arg(long)]
        file: String,
        #[arg(long)]
        dirty: String,
    },
    /// Response to diff prompt
    DiffResponse {
        #[arg(long)]
        id: String,
        #[arg(long)]
        accepted: bool,
    },
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Start { session, client, cwd } => {
            eprintln!("start: session={session} client={client} cwd={cwd}");
            todo!("daemon start")
        }
        Command::Send { session, msg } => {
            eprintln!("send: session={session}");
            todo!("client send")
        }
    }
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cd ~/Sites/rust/kak-claude && cargo build 2>&1`
Expected: Compiles successfully

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml src/main.rs
git commit -m "feat: project scaffolding with clap CLI"
```

---

### Task 2: JSON-RPC 2.0 and MCP protocol types

**Files:**
- Create: `src/mcp/mod.rs`
- Create: `src/mcp/protocol.rs`

- [ ] **Step 1: Write tests for protocol serialization**

Add to the bottom of `src/mcp/protocol.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serialize_jsonrpc_response() {
        let resp = JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: JsonRpcId::Number(1),
            result: Some(serde_json::json!({"protocolVersion": "2024-11-05"})),
            error: None,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"jsonrpc\":\"2.0\""));
        assert!(json.contains("\"id\":1"));
        assert!(json.contains("protocolVersion"));
    }

    #[test]
    fn test_deserialize_jsonrpc_request() {
        let json = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
        let req: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.method, "initialize");
        assert_eq!(req.id, Some(JsonRpcId::Number(1)));
    }

    #[test]
    fn test_deserialize_notification() {
        let json = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        let req: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.method, "notifications/initialized");
        assert!(req.id.is_none());
    }

    #[test]
    fn test_mcp_tool_response() {
        let inner = serde_json::json!({"success": true});
        let resp = mcp_tool_response(inner);
        // Should be double-encoded
        let content = resp.as_array().unwrap();
        assert_eq!(content.len(), 1);
        let text = content[0]["text"].as_str().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["success"], true);
    }

    #[test]
    fn test_jsonrpc_error() {
        let resp = JsonRpcResponse::error(JsonRpcId::Number(1), -32601, "Method not found");
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("-32601"));
        assert!(json.contains("Method not found"));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd ~/Sites/rust/kak-claude && cargo test -- mcp::protocol 2>&1`
Expected: FAIL — module and types don't exist

- [ ] **Step 3: Implement protocol types**

Create `src/mcp/mod.rs`:
```rust
pub mod protocol;
```

Create `src/mcp/protocol.rs`:
```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum JsonRpcId {
    Number(u64),
    String(String),
}

#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    #[serde(default)]
    pub id: Option<JsonRpcId>,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: JsonRpcId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
}

impl JsonRpcResponse {
    pub fn success(id: JsonRpcId, result: serde_json::Value) -> Self {
        Self { jsonrpc: "2.0".into(), id, result: Some(result), error: None }
    }

    pub fn error(id: JsonRpcId, code: i32, message: &str) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(JsonRpcError { code, message: message.into(), data: None }),
        }
    }
}

/// JSON-RPC notification (no id, no response expected)
#[derive(Debug, Serialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    pub params: serde_json::Value,
}

impl JsonRpcNotification {
    pub fn new(method: &str, params: serde_json::Value) -> Self {
        Self { jsonrpc: "2.0".into(), method: method.into(), params }
    }
}

/// Wrap a tool result in the MCP content envelope (double-encoded JSON)
pub fn mcp_tool_response(inner: serde_json::Value) -> serde_json::Value {
    serde_json::json!([{
        "type": "text",
        "text": serde_json::to_string(&inner).unwrap()
    }])
}

// Error codes
pub const PARSE_ERROR: i32 = -32700;
pub const INVALID_REQUEST: i32 = -32600;
pub const METHOD_NOT_FOUND: i32 = -32601;
pub const INVALID_PARAMS: i32 = -32602;
pub const INTERNAL_ERROR: i32 = -32603;
```

Add `mod mcp;` to `src/main.rs`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd ~/Sites/rust/kak-claude && cargo test -- mcp::protocol 2>&1`
Expected: All 5 tests PASS

- [ ] **Step 5: Commit**

```bash
git add src/mcp/
git commit -m "feat: JSON-RPC 2.0 and MCP protocol types with tests"
```

---

### Task 3: MCP tool schemas and registry

**Files:**
- Create: `src/mcp/tools.rs`

- [ ] **Step 1: Write test for tool list generation**

Add to bottom of `src/mcp/tools.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_list_has_all_tools() {
        let tools = tool_list();
        let names: Vec<&str> = tools.iter().map(|t| t.name).collect();
        assert!(names.contains(&"getCurrentSelection"));
        assert!(names.contains(&"getLatestSelection"));
        assert!(names.contains(&"getOpenEditors"));
        assert!(names.contains(&"getWorkspaceFolders"));
        assert!(names.contains(&"openFile"));
        assert!(names.contains(&"openDiff"));
        assert!(names.contains(&"checkDocumentDirty"));
        assert!(names.contains(&"saveDocument"));
        assert!(names.contains(&"closeAllDiffTabs"));
        assert_eq!(names.len(), 9);
    }

    #[test]
    fn test_tool_list_serializes() {
        let tools = tool_list();
        let json = serde_json::to_value(&tools).unwrap();
        let arr = json.as_array().unwrap();
        assert_eq!(arr.len(), 9);
        // Each tool should have name, description, inputSchema
        for tool in arr {
            assert!(tool["name"].is_string());
            assert!(tool["description"].is_string());
            assert!(tool["inputSchema"].is_object());
        }
    }

    #[test]
    fn test_open_file_schema_has_additional_properties_false() {
        let tools = tool_list();
        let open_file = tools.iter().find(|t| t.name == "openFile").unwrap();
        let schema = &open_file.input_schema;
        assert_eq!(schema["additionalProperties"], false);
        assert!(schema["required"].as_array().unwrap().contains(&serde_json::json!("filePath")));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd ~/Sites/rust/kak-claude && cargo test -- mcp::tools 2>&1`
Expected: FAIL

- [ ] **Step 3: Implement tool definitions**

```rust
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    #[serde(rename = "inputSchema")]
    pub input_schema: serde_json::Value,
}

fn empty_schema() -> serde_json::Value {
    serde_json::json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "type": "object",
        "additionalProperties": false
    })
}

fn file_path_schema() -> serde_json::Value {
    serde_json::json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "type": "object",
        "required": ["filePath"],
        "additionalProperties": false,
        "properties": {
            "filePath": { "type": "string", "description": "Path to the file" }
        }
    })
}

pub fn tool_list() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "getCurrentSelection",
            description: "Get the current text selection in the editor",
            input_schema: empty_schema(),
        },
        ToolDef {
            name: "getLatestSelection",
            description: "Get the most recent text selection (even if not in the active editor)",
            input_schema: empty_schema(),
        },
        ToolDef {
            name: "getOpenEditors",
            description: "Get list of currently open files",
            input_schema: empty_schema(),
        },
        ToolDef {
            name: "getWorkspaceFolders",
            description: "Get all workspace folders currently open in the IDE",
            input_schema: empty_schema(),
        },
        ToolDef {
            name: "openFile",
            description: "Open a file in the editor and optionally select a range of text",
            input_schema: serde_json::json!({
                "$schema": "http://json-schema.org/draft-07/schema#",
                "type": "object",
                "required": ["filePath"],
                "additionalProperties": false,
                "properties": {
                    "filePath": { "type": "string", "description": "Path to the file to open" },
                    "preview": { "type": "boolean", "description": "Open in preview mode", "default": false },
                    "startLine": { "type": "integer", "description": "Start line of selection" },
                    "endLine": { "type": "integer", "description": "End line of selection" },
                    "startText": { "type": "string", "description": "Text pattern to find selection start" },
                    "endText": { "type": "string", "description": "Text pattern to find selection end" },
                    "selectToEndOfLine": { "type": "boolean", "description": "Extend selection to end of line", "default": false },
                    "makeFrontmost": { "type": "boolean", "description": "Make file the active tab", "default": true }
                }
            }),
        },
        ToolDef {
            name: "openDiff",
            description: "Open a diff view comparing old file content with new file content",
            input_schema: serde_json::json!({
                "$schema": "http://json-schema.org/draft-07/schema#",
                "type": "object",
                "required": ["old_file_path", "new_file_path", "new_file_contents", "tab_name"],
                "additionalProperties": false,
                "properties": {
                    "old_file_path": { "type": "string", "description": "Path to the old file" },
                    "new_file_path": { "type": "string", "description": "Path to the new file" },
                    "new_file_contents": { "type": "string", "description": "Contents for the new file version" },
                    "tab_name": { "type": "string", "description": "Name for the diff tab/view" }
                }
            }),
        },
        ToolDef {
            name: "checkDocumentDirty",
            description: "Check if a document has unsaved changes",
            input_schema: file_path_schema(),
        },
        ToolDef {
            name: "saveDocument",
            description: "Save a document with unsaved changes",
            input_schema: file_path_schema(),
        },
        ToolDef {
            name: "closeAllDiffTabs",
            description: "Close all diff tabs in the editor",
            input_schema: empty_schema(),
        },
    ]
}
```

Add `pub mod tools;` to `src/mcp/mod.rs`.

- [ ] **Step 4: Run tests**

Run: `cd ~/Sites/rust/kak-claude && cargo test -- mcp::tools 2>&1`
Expected: All 3 tests PASS

- [ ] **Step 5: Commit**

```bash
git add src/mcp/
git commit -m "feat: MCP tool schemas and registry"
```

---

### Task 4: Kakoune state types

**Files:**
- Create: `src/kakoune/mod.rs`
- Create: `src/kakoune/state.rs`

- [ ] **Step 1: Write tests for state**

Add to bottom of `src/kakoune/state.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_update_selection() {
        let mut state = EditorState::new("/tmp/project".into());
        state.update_selection("hello world".into(), "/tmp/file.rs".into(), 10, 5);
        let sel = state.current_selection();
        assert_eq!(sel.text, "hello world");
        assert_eq!(sel.file_path, "/tmp/file.rs");
        assert_eq!(sel.line, 10);
        assert_eq!(sel.col, 5);
    }

    #[test]
    fn test_latest_selection_preserved() {
        let mut state = EditorState::new("/tmp/project".into());
        state.update_selection("selected text".into(), "/tmp/a.rs".into(), 5, 1);
        state.clear_current_selection();
        let current = state.current_selection();
        assert!(current.text.is_empty());
        let latest = state.latest_selection();
        assert_eq!(latest.text, "selected text");
    }

    #[test]
    fn test_parse_buflist() {
        let mut state = EditorState::new("/tmp".into());
        state.update_buffers("file1.rs:file2.rs:*debug*");
        assert_eq!(state.buffers().len(), 3);
        assert_eq!(state.buffers()[0].path, "file1.rs");
    }

    #[test]
    fn test_selection_to_mcp_json() {
        let mut state = EditorState::new("/tmp".into());
        state.update_selection("hi".into(), "/tmp/f.rs".into(), 3, 7);
        let json = state.current_selection().to_mcp_json();
        assert_eq!(json["filePath"], "/tmp/f.rs");
        assert_eq!(json["fileUrl"], "file:///tmp/f.rs");
        assert_eq!(json["selection"]["start"]["line"], 2); // 0-based
        assert_eq!(json["selection"]["start"]["character"], 6); // 0-based
        assert!(!json["selection"]["isEmpty"].as_bool().unwrap());
    }

    #[test]
    fn test_empty_selection_is_empty() {
        let sel = Selection::empty("/tmp/f.rs", 1, 1);
        let json = sel.to_mcp_json();
        assert!(json["selection"]["isEmpty"].as_bool().unwrap());
        assert!(json["text"].as_str().unwrap().is_empty());
    }

    #[test]
    fn test_workspace_folders_json() {
        let state = EditorState::new("/home/user/project".into());
        let json = state.workspace_folders_json();
        assert_eq!(json["rootPath"], "/home/user/project");
        assert_eq!(json["folders"][0]["path"], "/home/user/project");
    }

    #[test]
    fn test_open_editors_json() {
        let mut state = EditorState::new("/tmp".into());
        state.update_buffers("src/main.rs:src/lib.rs");
        let json = state.open_editors_json();
        let tabs = json["tabs"].as_array().unwrap();
        assert_eq!(tabs.len(), 2);
        assert!(tabs[0]["uri"].as_str().unwrap().contains("main.rs"));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd ~/Sites/rust/kak-claude && cargo test -- kakoune::state 2>&1`
Expected: FAIL

- [ ] **Step 3: Implement state types**

Create `src/kakoune/mod.rs`:
```rust
pub mod state;
pub mod session;
pub mod socket;
```

Create `src/kakoune/state.rs`:
```rust
use serde_json;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct Selection {
    pub text: String,
    pub file_path: String,
    pub line: u32,   // 1-based (Kakoune native)
    pub col: u32,    // 1-based (Kakoune native)
}

impl Selection {
    pub fn empty(file_path: &str, line: u32, col: u32) -> Self {
        Self { text: String::new(), file_path: file_path.into(), line, col }
    }

    /// Convert to MCP JSON with 0-based positions
    pub fn to_mcp_json(&self) -> serde_json::Value {
        let line_0 = if self.line > 0 { self.line - 1 } else { 0 };
        let col_0 = if self.col > 0 { self.col - 1 } else { 0 };
        let is_empty = self.text.is_empty();

        // Estimate end position from text content
        let (end_line, end_col) = if is_empty {
            (line_0, col_0)
        } else {
            let lines: Vec<&str> = self.text.split('\n').collect();
            let end_l = line_0 + (lines.len() as u32) - 1;
            let end_c = if lines.len() == 1 {
                col_0 + lines[0].len() as u32
            } else {
                lines.last().unwrap().len() as u32
            };
            (end_l, end_c)
        };

        serde_json::json!({
            "text": self.text,
            "filePath": self.file_path,
            "fileUrl": format!("file://{}", self.file_path),
            "selection": {
                "start": { "line": line_0, "character": col_0 },
                "end": { "line": end_line, "character": end_col },
                "isEmpty": is_empty
            }
        })
    }
}

#[derive(Debug, Clone)]
pub struct BufferInfo {
    pub path: String,
    pub is_active: bool,
}

pub struct EditorState {
    cwd: String,
    current: Selection,
    latest: Selection,
    buffers: Vec<BufferInfo>,
}

impl EditorState {
    pub fn new(cwd: String) -> Self {
        Self {
            current: Selection::empty("", 0, 0),
            latest: Selection::empty("", 0, 0),
            buffers: Vec::new(),
            cwd,
        }
    }

    pub fn update_selection(&mut self, text: String, file: String, line: u32, col: u32) {
        self.current = Selection { text: text.clone(), file_path: file.clone(), line, col };
        if !text.is_empty() {
            self.latest = self.current.clone();
        }
    }

    pub fn clear_current_selection(&mut self) {
        self.current = Selection::empty(&self.current.file_path, self.current.line, self.current.col);
    }

    pub fn current_selection(&self) -> &Selection {
        &self.current
    }

    pub fn latest_selection(&self) -> &Selection {
        &self.latest
    }

    pub fn update_buffers(&mut self, buflist: &str) {
        self.buffers = buflist
            .split(':')
            .filter(|s| !s.is_empty())
            .map(|s| BufferInfo { path: s.to_string(), is_active: false })
            .collect();
        // Mark first buffer as active (simplification)
        if let Some(first) = self.buffers.first_mut() {
            first.is_active = true;
        }
    }

    pub fn buffers(&self) -> &[BufferInfo] {
        &self.buffers
    }

    pub fn cwd(&self) -> &str {
        &self.cwd
    }

    pub fn workspace_folders_json(&self) -> serde_json::Value {
        let name = Path::new(&self.cwd)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| self.cwd.clone());
        serde_json::json!({
            "success": true,
            "folders": [{
                "name": name,
                "uri": format!("file://{}", self.cwd),
                "path": self.cwd
            }],
            "rootPath": self.cwd
        })
    }

    pub fn open_editors_json(&self) -> serde_json::Value {
        let tabs: Vec<serde_json::Value> = self.buffers.iter().map(|b| {
            let file_name = Path::new(&b.path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| b.path.clone());
            let lang = guess_language(&b.path);
            serde_json::json!({
                "uri": format!("file://{}", if b.path.starts_with('/') { &b.path } else { &format!("{}/{}", self.cwd, b.path) }),
                "isActive": b.is_active,
                "isDirty": false,
                "label": file_name,
                "languageId": lang,
                "lineCount": 0,
                "fileName": file_name
            })
        }).collect();
        serde_json::json!({ "tabs": tabs })
    }
}

fn guess_language(path: &str) -> &'static str {
    match Path::new(path).extension().and_then(|e| e.to_str()) {
        Some("rs") => "rust",
        Some("js") => "javascript",
        Some("ts") => "typescript",
        Some("tsx") => "typescriptreact",
        Some("jsx") => "javascriptreact",
        Some("py") => "python",
        Some("rb") => "ruby",
        Some("go") => "go",
        Some("c") => "c",
        Some("cpp" | "cc" | "cxx") => "cpp",
        Some("h" | "hpp") => "cpp",
        Some("java") => "java",
        Some("kt") => "kotlin",
        Some("swift") => "swift",
        Some("sh" | "bash" | "zsh") => "shellscript",
        Some("html") => "html",
        Some("css") => "css",
        Some("json") => "json",
        Some("yaml" | "yml") => "yaml",
        Some("toml") => "toml",
        Some("md") => "markdown",
        Some("lua") => "lua",
        Some("zig") => "zig",
        Some("nix") => "nix",
        Some("kak") => "kakoune",
        _ => "plaintext",
    }
}
```

Add `mod kakoune;` to `src/main.rs`.

- [ ] **Step 4: Run tests**

Run: `cd ~/Sites/rust/kak-claude && cargo test -- kakoune::state 2>&1`
Expected: All 7 tests PASS

- [ ] **Step 5: Commit**

```bash
git add src/kakoune/
git commit -m "feat: editor state types with selection tracking and MCP JSON conversion"
```

---

### Task 5: Lock file management

**Files:**
- Create: `src/lockfile.rs`

- [ ] **Step 1: Write tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_lockfile_create_and_read() {
        let dir = TempDir::new().unwrap();
        let lf = LockFile::create_in(dir.path(), 12345, 9876, &["/tmp/project"]).unwrap();
        assert!(lf.path.exists());
        let contents: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&lf.path).unwrap()).unwrap();
        assert_eq!(contents["pid"], 12345);
        assert_eq!(contents["ideName"], "Kakoune");
        assert_eq!(contents["transport"], "ws");
        assert!(contents["authToken"].as_str().unwrap().len() > 10);
    }

    #[test]
    fn test_lockfile_cleanup_on_drop() {
        let dir = TempDir::new().unwrap();
        let path;
        {
            let lf = LockFile::create_in(dir.path(), 111, 8080, &["/tmp"]).unwrap();
            path = lf.path.clone();
            assert!(path.exists());
        }
        assert!(!path.exists());
    }

    #[test]
    fn test_auth_token_is_uuid() {
        let dir = TempDir::new().unwrap();
        let lf = LockFile::create_in(dir.path(), 1, 80, &["/tmp"]).unwrap();
        assert!(lf.auth_token.len() == 36); // UUID v4 format
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd ~/Sites/rust/kak-claude && cargo test -- lockfile 2>&1`
Expected: FAIL

- [ ] **Step 3: Implement lockfile**

```rust
use serde_json;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub struct LockFile {
    pub path: PathBuf,
    pub auth_token: String,
}

impl LockFile {
    /// Create lock file using default config dir ($CLAUDE_CONFIG_DIR or ~/.claude)
    pub fn create(pid: u32, port: u16, workspace_folders: &[&str]) -> std::io::Result<Self> {
        Self::create_in(&config_dir(), pid, port, workspace_folders)
    }

    /// Create lock file in a specific base directory (testable, no env vars)
    pub fn create_in(base_dir: &Path, pid: u32, port: u16, workspace_folders: &[&str]) -> std::io::Result<Self> {
        let auth_token = Uuid::new_v4().to_string();
        let ide_dir = base_dir.join("ide");
        fs::create_dir_all(&ide_dir)?;

        let path = ide_dir.join(format!("{}.lock", port));
        let content = serde_json::json!({
            "pid": pid,
            "workspaceFolders": workspace_folders,
            "ideName": "Kakoune",
            "transport": "ws",
            "authToken": auth_token
        });
        fs::write(&path, serde_json::to_string_pretty(&content).unwrap())?;

        Ok(Self { path, auth_token })
    }
}

impl Drop for LockFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn config_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        PathBuf::from(dir)
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".claude")
    } else {
        PathBuf::from("/tmp/.claude")
    }
}
```

Add `mod lockfile;` to `src/main.rs`.

- [ ] **Step 4: Run tests**

Run: `cd ~/Sites/rust/kak-claude && cargo test -- lockfile 2>&1`
Expected: All 3 tests PASS

- [ ] **Step 5: Commit**

```bash
git add src/lockfile.rs
git commit -m "feat: lock file management with cleanup on drop"
```

---

### Task 6: Client (send subcommand)

**Files:**
- Create: `src/client.rs`

- [ ] **Step 1: Write test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_state_message() {
        let msg = build_state_message("/tmp/f.rs", 10, 5, "hello");
        let parsed: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(parsed["type"], "state");
        assert_eq!(parsed["file"], "/tmp/f.rs");
        assert_eq!(parsed["line"], 10);
        assert_eq!(parsed["col"], 5);
        assert_eq!(parsed["selection"], "hello");
    }

    #[test]
    fn test_build_buffers_message() {
        let msg = build_buffers_message("a.rs:b.rs");
        let parsed: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(parsed["type"], "buffers");
        assert_eq!(parsed["list"], "a.rs:b.rs");
    }

    #[test]
    fn test_build_shutdown_message() {
        let msg = build_shutdown_message();
        let parsed: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(parsed["type"], "shutdown");
    }

    #[test]
    fn test_state_message_escapes_special_chars() {
        let msg = build_state_message("/tmp/f.rs", 1, 1, "line with \"quotes\" and \nnewline");
        let parsed: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(parsed["selection"], "line with \"quotes\" and \nnewline");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd ~/Sites/rust/kak-claude && cargo test -- client 2>&1`
Expected: FAIL

- [ ] **Step 3: Implement client**

```rust
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

pub fn build_state_message(file: &str, line: u32, col: u32, selection: &str) -> String {
    serde_json::json!({
        "type": "state",
        "file": file,
        "line": line,
        "col": col,
        "selection": selection
    }).to_string()
}

pub fn build_buffers_message(list: &str) -> String {
    serde_json::json!({
        "type": "buffers",
        "list": list
    }).to_string()
}

pub fn build_shutdown_message() -> String {
    serde_json::json!({ "type": "shutdown" }).to_string()
}

pub fn build_dirty_response(file: &str, dirty: &str) -> String {
    serde_json::json!({
        "type": "dirty-response",
        "file": file,
        "dirty": dirty == "true"
    }).to_string()
}

pub fn build_diff_response(id: &str, accepted: bool) -> String {
    serde_json::json!({
        "type": "diff-response",
        "id": id,
        "accepted": accepted
    }).to_string()
}

pub fn send_message(session: &str, message: &str) -> std::io::Result<()> {
    let socket_path = socket_path(session);
    let mut stream = UnixStream::connect(&socket_path)?;
    stream.write_all(message.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    Ok(())
}

fn socket_path(session: &str) -> PathBuf {
    let tmpdir = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(tmpdir)
        .join("kak-claude")
        .join(session)
        .join("sock")
}
```

Add `mod client;` to `src/main.rs`.

- [ ] **Step 4: Run tests**

Run: `cd ~/Sites/rust/kak-claude && cargo test -- client 2>&1`
Expected: All 4 tests PASS

- [ ] **Step 5: Wire send subcommand in main.rs**

Update the `Command::Send` match arm in `main.rs`:

```rust
Command::Send { session, msg } => {
    let message = match msg {
        SendMessage::State { file, line, col, selection } => {
            client::build_state_message(&file, line, col, &selection)
        }
        SendMessage::Buffers { list } => client::build_buffers_message(&list),
        SendMessage::Shutdown => client::build_shutdown_message(),
        SendMessage::DirtyResponse { file, dirty } => {
            client::build_dirty_response(&file, &dirty)
        }
        SendMessage::DiffResponse { id, accepted } => {
            client::build_diff_response(&id, accepted)
        }
    };
    if let Err(e) = client::send_message(&session, &message) {
        eprintln!("Failed to send: {e}");
        std::process::exit(1);
    }
}
```

- [ ] **Step 6: Verify it compiles**

Run: `cd ~/Sites/rust/kak-claude && cargo build 2>&1`
Expected: Compiles

- [ ] **Step 7: Commit**

```bash
git add src/client.rs src/main.rs
git commit -m "feat: client send subcommand with message builders"
```

---

### Task 7: Kakoune session sender (kak -p)

**Files:**
- Create: `src/kakoune/session.rs`

- [ ] **Step 1: Implement session sender**

```rust
use std::io::Write;
use std::process::{Command, Stdio};

pub struct KakSession {
    session: String,
    client: String,
}

impl KakSession {
    pub fn new(session: String, client: String) -> Self {
        Self { session, client }
    }

    pub fn session_name(&self) -> &str {
        &self.session
    }

    pub fn client_name(&self) -> &str {
        &self.client
    }

    /// Send a command to Kakoune, targeting the stored client
    pub fn eval(&self, command: &str) -> std::io::Result<()> {
        let full_cmd = format!("evaluate-commands -client {} -- %§{}§", self.client, command);
        self.send_raw(&full_cmd)
    }

    /// Send a raw command to the Kakoune session (no client targeting)
    pub fn send_raw(&self, command: &str) -> std::io::Result<()> {
        let mut child = Command::new("kak")
            .arg("-p")
            .arg(&self.session)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(command.as_bytes())?;
        }

        child.wait()?;
        Ok(())
    }

    /// Open a file in the editor
    pub fn open_file(&self, path: &str) -> std::io::Result<()> {
        self.eval(&format!("edit '{}'", path.replace('\'', "''")))
    }

    /// Open a file and select a line range
    pub fn open_file_at(&self, path: &str, start_line: u32, end_line: Option<u32>) -> std::io::Result<()> {
        let end = end_line.unwrap_or(start_line);
        self.eval(&format!(
            "edit '{}'; execute-keys {}g{}G",
            path.replace('\'', "''"),
            start_line,
            end
        ))
    }

    /// Build the eval command string (exposed for testing)
    pub fn build_eval(&self, command: &str) -> String {
        format!("evaluate-commands -client {} -- %[{}]", self.client, command)
    }

    /// Show diff in a fifo buffer and prompt for accept/reject
    pub fn show_diff(&self, old_path: &str, new_path: &str, request_id: &str, width: u32) -> std::io::Result<()> {
        let escaped_old = old_path.replace('\'', "''");
        let escaped_new = new_path.replace('\'', "''");
        // Use %[ ] for outer Kakoune blocks, %sh[ ] for shell blocks
        // Kakoune supports matching bracket delimiters for nesting
        let cmd = format!(
            concat!(
                "fifo -name '*claude-diff*' -scroll -- difft --width {} --color always '{}' '{}'\n",
                "hook -once buffer BufCloseFifo .* %[",
                "  prompt 'Accept changes? (y/n): ' %[",
                "    nop %sh[",
                "      case \"$kak_text\" in",
                "        y*) kak-claude send --session \"$kak_session\" diff-response --id '{}' --accepted true ;;",
                "        *)  kak-claude send --session \"$kak_session\" diff-response --id '{}' --accepted false ;;",
                "      esac",
                "    ]",
                "  ]",
                "]"
            ),
            width, escaped_old, escaped_new, request_id, request_id,
        );
        self.eval(&cmd)
    }

    /// Close all diff buffers
    pub fn close_diff_buffers(&self) -> std::io::Result<()> {
        self.eval("try %[ evaluate-commands -buffer '*claude-diff*' delete-buffer ]")
    }

    /// Save a buffer
    pub fn save_buffer(&self, path: &str) -> std::io::Result<()> {
        self.eval(&format!(
            "evaluate-commands -buffer '{}' write",
            path.replace('\'', "''")
        ))
    }

    /// Query if a buffer is dirty (response comes back via Unix socket)
    pub fn query_dirty(&self, path: &str) -> std::io::Result<()> {
        let escaped = path.replace('\'', "''");
        let cmd = format!(
            concat!(
                "evaluate-commands -buffer '{}' %[",
                "  nop %sh[ kak-claude send --session \"$kak_session\" dirty-response --file '{}' --dirty \"$kak_modified\" ]",
                "]"
            ),
            escaped, escaped,
        );
        self.eval(&cmd)
    }
}
```

- [ ] **Step 2: Write tests for command building**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_eval() {
        let kak = KakSession::new("test-session".into(), "main".into());
        let cmd = kak.build_eval("edit foo.rs");
        assert_eq!(cmd, "evaluate-commands -client main -- %[edit foo.rs]");
    }

    #[test]
    fn test_session_accessors() {
        let kak = KakSession::new("sess".into(), "cli".into());
        assert_eq!(kak.session_name(), "sess");
        assert_eq!(kak.client_name(), "cli");
    }

    #[test]
    fn test_open_file_escapes_quotes() {
        let kak = KakSession::new("s".into(), "c".into());
        let cmd = kak.build_eval(&format!("edit '{}'", "it's".replace('\'', "''")));
        assert!(cmd.contains("it''s"));
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cd ~/Sites/rust/kak-claude && cargo test -- kakoune::session 2>&1`
Expected: All 3 tests PASS

- [ ] **Step 4: Verify it compiles**

Run: `cd ~/Sites/rust/kak-claude && cargo build 2>&1`
Expected: Compiles

- [ ] **Step 3: Commit**

```bash
git add src/kakoune/session.rs
git commit -m "feat: Kakoune session sender (kak -p) with file/diff/save operations"
```

---

## Chunk 2: Server Core (mio Event Loop + WebSocket)

### Task 8: Unix socket listener

**Files:**
- Create: `src/kakoune/socket.rs`

- [ ] **Step 1: Write test for message parsing**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_state_message() {
        let msg = r#"{"type":"state","file":"/tmp/f.rs","line":10,"col":5,"selection":"hello"}"#;
        let parsed = KakMessage::parse(msg).unwrap();
        match parsed {
            KakMessage::State { file, line, col, selection } => {
                assert_eq!(file, "/tmp/f.rs");
                assert_eq!(line, 10);
                assert_eq!(col, 5);
                assert_eq!(selection, "hello");
            }
            _ => panic!("Expected State"),
        }
    }

    #[test]
    fn test_parse_buffers_message() {
        let msg = r#"{"type":"buffers","list":"a.rs:b.rs"}"#;
        let parsed = KakMessage::parse(msg).unwrap();
        match parsed {
            KakMessage::Buffers { list } => assert_eq!(list, "a.rs:b.rs"),
            _ => panic!("Expected Buffers"),
        }
    }

    #[test]
    fn test_parse_shutdown_message() {
        let msg = r#"{"type":"shutdown"}"#;
        let parsed = KakMessage::parse(msg).unwrap();
        assert!(matches!(parsed, KakMessage::Shutdown));
    }

    #[test]
    fn test_parse_diff_response() {
        let msg = r#"{"type":"diff-response","id":"abc-123","accepted":true}"#;
        let parsed = KakMessage::parse(msg).unwrap();
        match parsed {
            KakMessage::DiffResponse { id, accepted } => {
                assert_eq!(id, "abc-123");
                assert!(accepted);
            }
            _ => panic!("Expected DiffResponse"),
        }
    }

    #[test]
    fn test_parse_invalid_json() {
        assert!(KakMessage::parse("not json").is_err());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd ~/Sites/rust/kak-claude && cargo test -- kakoune::socket 2>&1`
Expected: FAIL

- [ ] **Step 3: Implement socket message types**

```rust
use serde::Deserialize;

#[derive(Debug)]
pub enum KakMessage {
    State { file: String, line: u32, col: u32, selection: String },
    Buffers { list: String },
    Shutdown,
    DirtyResponse { file: String, dirty: bool },
    DiffResponse { id: String, accepted: bool },
}

#[derive(Deserialize)]
struct RawMessage {
    #[serde(rename = "type")]
    msg_type: String,
    #[serde(default)]
    file: String,
    #[serde(default)]
    line: u32,
    #[serde(default)]
    col: u32,
    #[serde(default)]
    selection: String,
    #[serde(default)]
    list: String,
    #[serde(default)]
    dirty: bool,
    #[serde(default)]
    id: String,
    #[serde(default)]
    accepted: bool,
}

impl KakMessage {
    pub fn parse(input: &str) -> Result<Self, String> {
        let raw: RawMessage = serde_json::from_str(input)
            .map_err(|e| format!("Invalid JSON: {e}"))?;

        match raw.msg_type.as_str() {
            "state" => Ok(KakMessage::State {
                file: raw.file,
                line: raw.line,
                col: raw.col,
                selection: raw.selection,
            }),
            "buffers" => Ok(KakMessage::Buffers { list: raw.list }),
            "shutdown" => Ok(KakMessage::Shutdown),
            "dirty-response" => Ok(KakMessage::DirtyResponse {
                file: raw.file,
                dirty: raw.dirty,
            }),
            "diff-response" => Ok(KakMessage::DiffResponse {
                id: raw.id,
                accepted: raw.accepted,
            }),
            other => Err(format!("Unknown message type: {other}")),
        }
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cd ~/Sites/rust/kak-claude && cargo test -- kakoune::socket 2>&1`
Expected: All 5 tests PASS

- [ ] **Step 5: Commit**

```bash
git add src/kakoune/socket.rs
git commit -m "feat: Unix socket message types with parsing"
```

---

### Task 9: WebSocket server with mio

**Files:**
- Create: `src/websocket.rs`

- [ ] **Step 1: Implement WebSocket connection handler**

This module wraps tungstenite over mio's non-blocking TcpStream:

```rust
use mio::net::TcpStream;
use std::io;
use tungstenite::handshake::server::{NoCallback, ServerHandshake};
use tungstenite::handshake::MidHandshake;
use tungstenite::{HandshakeError, Message, WebSocket};

pub enum WsState {
    /// Waiting for WebSocket handshake
    Pending(TcpStream),
    /// Mid-handshake (non-blocking retry needed)
    Handshaking(MidHandshake<ServerHandshake<TcpStream, NoCallback>>),
    /// Fully connected WebSocket
    Connected(WebSocket<TcpStream>),
    /// Connection closed
    Closed,
}

pub struct WsConnection {
    state: WsState,
    write_queue: std::collections::VecDeque<Message>,
    pub authenticated: bool,
}

impl WsConnection {
    pub fn new(stream: TcpStream) -> Self {
        Self {
            state: WsState::Pending(stream),
            write_queue: std::collections::VecDeque::new(),
            authenticated: false,
        }
    }

    /// Attempt or continue WebSocket handshake.
    /// Returns Ok(true) when handshake is complete, Ok(false) when still in progress.
    pub fn try_handshake(&mut self, auth_token: &str) -> Result<bool, String> {
        let state = std::mem::replace(&mut self.state, WsState::Closed);

        match state {
            WsState::Pending(stream) => {
                // Start handshake with auth validation via callback
                let token = auth_token.to_string();
                let callback = move |req: &tungstenite::handshake::server::Request,
                                     resp: tungstenite::handshake::server::Response|
                      -> Result<tungstenite::handshake::server::Response, tungstenite::handshake::server::ErrorResponse> {
                    let auth = req.headers().get("x-claude-code-ide-authorization");
                    match auth {
                        Some(val) if val.to_str().unwrap_or("") == token => Ok(resp),
                        _ => {
                            let mut resp = tungstenite::http::Response::builder()
                                .status(400)
                                .body(None)
                                .unwrap();
                            Err(resp)
                        }
                    }
                };

                match tungstenite::accept_hdr(stream, callback) {
                    Ok(ws) => {
                        self.state = WsState::Connected(ws);
                        self.authenticated = true;
                        Ok(true)
                    }
                    Err(HandshakeError::Interrupted(mid)) => {
                        self.state = WsState::Handshaking(mid);
                        Ok(false)
                    }
                    Err(HandshakeError::Failure(e)) => {
                        self.state = WsState::Closed;
                        Err(format!("Handshake failed: {e}"))
                    }
                }
            }
            WsState::Handshaking(mid) => {
                match mid.handshake() {
                    Ok(ws) => {
                        self.state = WsState::Connected(ws);
                        self.authenticated = true;
                        Ok(true)
                    }
                    Err(HandshakeError::Interrupted(mid)) => {
                        self.state = WsState::Handshaking(mid);
                        Ok(false)
                    }
                    Err(HandshakeError::Failure(e)) => {
                        self.state = WsState::Closed;
                        Err(format!("Handshake failed: {e}"))
                    }
                }
            }
            WsState::Connected(ws) => {
                self.state = WsState::Connected(ws);
                Ok(true)
            }
            WsState::Closed => Err("Connection closed".into()),
        }
    }

    /// Read a message, returns None on WouldBlock
    pub fn read_message(&mut self) -> Result<Option<String>, WsError> {
        match &mut self.state {
            WsState::Connected(ws) => match ws.read() {
                Ok(Message::Text(text)) => Ok(Some(text.to_string())),
                Ok(Message::Ping(data)) => {
                    let _ = ws.send(Message::Pong(data));
                    Ok(None)
                }
                Ok(Message::Pong(_)) => Ok(None),
                Ok(Message::Close(_)) => Err(WsError::Closed),
                Ok(_) => Ok(None),
                Err(tungstenite::Error::Io(ref e)) if e.kind() == io::ErrorKind::WouldBlock => {
                    Ok(None)
                }
                Err(tungstenite::Error::ConnectionClosed) => Err(WsError::Closed),
                Err(e) => Err(WsError::Other(format!("{e}"))),
            },
            _ => Err(WsError::NotConnected),
        }
    }

    /// Queue a message to send
    pub fn queue_message(&mut self, text: &str) {
        self.write_queue.push_back(Message::Text(text.into()));
    }

    /// Flush queued messages, returns false if connection closed
    pub fn flush(&mut self) -> bool {
        if let WsState::Connected(ws) = &mut self.state {
            while let Some(msg) = self.write_queue.pop_front() {
                match ws.send(msg.clone()) {
                    Ok(_) => {}
                    Err(tungstenite::Error::Io(ref e)) if e.kind() == io::ErrorKind::WouldBlock => {
                        self.write_queue.push_front(msg); // Put it back
                        return true; // Retry later
                    }
                    Err(_) => return false,
                }
            }
            true
        } else {
            false
        }
    }

    /// Send a ping frame
    pub fn ping(&mut self) -> bool {
        if let WsState::Connected(ws) = &mut self.state {
            ws.send(Message::Ping(vec![].into())).is_ok()
        } else {
            false
        }
    }

    /// Get mutable reference to underlying TcpStream for mio registration
    pub fn tcp_stream_mut(&mut self) -> Option<&mut TcpStream> {
        match &mut self.state {
            WsState::Pending(s) => Some(s),
            WsState::Handshaking(mid) => Some(mid.get_mut().get_mut()),
            WsState::Connected(ws) => Some(ws.get_mut()),
            WsState::Closed => None,
        }
    }

    pub fn is_connected(&self) -> bool {
        matches!(self.state, WsState::Connected(_))
    }

    pub fn is_closed(&self) -> bool {
        matches!(self.state, WsState::Closed)
    }
}

#[derive(Debug)]
pub enum WsError {
    Closed,
    NotConnected,
    Other(String),
}
```

Add `mod websocket;` to `src/main.rs`.

- [ ] **Step 2: Verify it compiles**

Run: `cd ~/Sites/rust/kak-claude && cargo build 2>&1`
Expected: Compiles (may need to adjust tungstenite API for version 0.24)

- [ ] **Step 3: Commit**

```bash
git add src/websocket.rs
git commit -m "feat: WebSocket connection handler with mio non-blocking support"
```

---

### Task 10: Main server event loop

**Files:**
- Create: `src/server.rs`

- [ ] **Step 1: Implement the mio event loop**

```rust
use crate::kakoune::session::KakSession;
use crate::kakoune::socket::KakMessage;
use crate::kakoune::state::EditorState;
use crate::lockfile::LockFile;
use crate::mcp::protocol::*;
use crate::mcp::tools;
use crate::websocket::{WsConnection, WsError};
use mio::net::{TcpListener, UnixListener};
use mio::{Events, Interest, Poll, Token};
use std::collections::HashMap;
use std::io::{self, Read};
use std::time::{Duration, Instant};

const UNIX_LISTENER: Token = Token(0);
const TCP_LISTENER: Token = Token(1);
const TOKEN_START: usize = 2;

pub struct Server {
    poll: Poll,
    unix_listener: UnixListener,
    tcp_listener: TcpListener,
    state: EditorState,
    kak: KakSession,
    lockfile: LockFile,
    ws_connections: HashMap<Token, WsConnection>,
    unix_streams: HashMap<Token, mio::net::UnixStream>,
    unix_buffers: HashMap<Token, Vec<u8>>,
    next_token: usize,
    pending_dirty: HashMap<String, (JsonRpcId, Token)>,
    pending_diff: HashMap<String, (JsonRpcId, Token)>,
    /// Token of the most recently active WebSocket client (for targeting responses)
    active_ws_token: Option<Token>,
    last_ping: Instant,
    should_quit: bool,
}

impl Server {
    pub fn new(session: &str, client: &str, cwd: &str) -> io::Result<Self> {
        let poll = Poll::new()?;

        // Unix socket
        let tmpdir = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
        let session_dir = std::path::PathBuf::from(&tmpdir)
            .join("kak-claude")
            .join(session);
        std::fs::create_dir_all(&session_dir)?;
        let sock_path = session_dir.join("sock");
        let _ = std::fs::remove_file(&sock_path); // Remove stale socket
        let mut unix_listener = UnixListener::bind(&sock_path)?;
        poll.registry().register(&mut unix_listener, UNIX_LISTENER, Interest::READABLE)?;

        // TCP socket for WebSocket (random port)
        let tcp_addr: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();
        let mut tcp_listener = TcpListener::bind(tcp_addr)?;
        let port = tcp_listener.local_addr()?.port();
        poll.registry().register(&mut tcp_listener, TCP_LISTENER, Interest::READABLE)?;

        // Write port file
        let port_file = session_dir.join("port");
        std::fs::write(&port_file, port.to_string())?;

        // Write PID file
        let pid_file = session_dir.join("pid");
        std::fs::write(&pid_file, std::process::id().to_string())?;

        // Lock file
        let lockfile = LockFile::create(std::process::id(), port, &[cwd])
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        let kak = KakSession::new(session.into(), client.into());
        let state = EditorState::new(cwd.into());

        Ok(Self {
            poll,
            unix_listener,
            tcp_listener,
            state,
            kak,
            lockfile,
            ws_connections: HashMap::new(),
            unix_streams: HashMap::new(),
            unix_buffers: HashMap::new(),
            next_token: TOKEN_START,
            pending_dirty: HashMap::new(),
            pending_diff: HashMap::new(),
            active_ws_token: None,
            last_ping: Instant::now(),
            should_quit: false,
        })
    }

    pub fn port(&self) -> u16 {
        self.tcp_listener.local_addr().unwrap().port()
    }

    fn next_token(&mut self) -> Token {
        let token = Token(self.next_token);
        self.next_token += 1;
        token
    }

    pub fn run(&mut self) -> io::Result<()> {
        let mut events = Events::with_capacity(64);
        let timeout = Some(Duration::from_secs(5)); // For ping timer

        while !self.should_quit {
            self.poll.poll(&mut events, timeout)?;

            // Ping timer
            if self.last_ping.elapsed() >= Duration::from_secs(30) {
                self.send_pings();
                self.last_ping = Instant::now();
            }

            for event in events.iter() {
                match event.token() {
                    UNIX_LISTENER => self.accept_unix_connections()?,
                    TCP_LISTENER => self.accept_tcp_connections()?,
                    token => {
                        if self.ws_connections.contains_key(&token) {
                            self.handle_ws_event(token);
                        } else if self.unix_buffers.contains_key(&token) {
                            self.handle_unix_event(token);
                        }
                    }
                }
            }
        }

        self.cleanup();
        Ok(())
    }

    fn accept_unix_connections(&mut self) -> io::Result<()> {
        loop {
            match self.unix_listener.accept() {
                Ok((mut stream, _)) => {
                    let token = self.next_token();
                    self.poll.registry().register(
                        &mut stream,
                        token,
                        Interest::READABLE,
                    )?;
                    self.unix_streams.insert(token, stream);
                    self.unix_buffers.insert(token, Vec::new());
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    fn accept_tcp_connections(&mut self) -> io::Result<()> {
        loop {
            match self.tcp_listener.accept() {
                Ok((stream, _)) => {
                    let token = self.next_token();
                    let mut conn = WsConnection::new(stream);
                    if let Some(tcp) = conn.tcp_stream_mut() {
                        self.poll.registry().register(
                            tcp,
                            token,
                            Interest::READABLE | Interest::WRITABLE,
                        )?;
                    }
                    self.ws_connections.insert(token, conn);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    fn handle_ws_event(&mut self, token: Token) {
        // Try handshake if needed
        let auth_token = self.lockfile.auth_token.clone();
        if let Some(conn) = self.ws_connections.get_mut(&token) {
            if !conn.is_connected() {
                match conn.try_handshake(&auth_token) {
                    Ok(true) => {} // Handshake complete
                    Ok(false) => return, // Still handshaking
                    Err(_) => {
                        self.ws_connections.remove(&token);
                        return;
                    }
                }
            }
        }

        // Collect messages first (avoids borrow conflict with self.handle_mcp_message)
        let mut messages = Vec::new();
        let mut closed = false;
        if let Some(conn) = self.ws_connections.get_mut(&token) {
            loop {
                match conn.read_message() {
                    Ok(Some(text)) => messages.push(text),
                    Ok(None) => break,
                    Err(WsError::Closed) => { closed = true; break; }
                    Err(_) => break,
                }
            }
        }
        if closed { self.ws_connections.remove(&token); return; }

        // Process messages (no borrow on ws_connections)
        let mut responses = Vec::new();
        for text in &messages {
            if let Some(resp) = self.handle_mcp_message(text) {
                responses.push(resp);
            }
        }

        // Queue responses and flush
        if let Some(conn) = self.ws_connections.get_mut(&token) {
            for resp in responses {
                conn.queue_message(&resp);
            }
            if !conn.flush() {
                self.ws_connections.remove(&token);
            }
        }
    }

    fn handle_unix_event(&mut self, token: Token) {
        let mut should_remove = false;
        if let Some(stream) = self.unix_streams.get_mut(&token) {
            let buf = self.unix_buffers.entry(token).or_default();
            let mut tmp = [0u8; 4096];
            loop {
                match stream.read(&mut tmp) {
                    Ok(0) => { should_remove = true; break; }
                    Ok(n) => buf.extend_from_slice(&tmp[..n]),
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(_) => { should_remove = true; break; }
                }
            }
        }

        // Process complete messages (newline-delimited)
        if let Some(buf) = self.unix_buffers.get_mut(&token) {
            while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = buf.drain(..=pos).collect();
                if let Ok(text) = std::str::from_utf8(&line[..line.len()-1]) {
                    if let Ok(msg) = KakMessage::parse(text) {
                        self.process_kak_message(msg);
                    }
                }
            }
        }

        if should_remove {
            self.unix_streams.remove(&token);
            self.unix_buffers.remove(&token);
        }
    }

    fn handle_mcp_message(&mut self, text: &str) -> Option<String> {
        let req: JsonRpcRequest = match serde_json::from_str(text) {
            Ok(r) => r,
            Err(_) => return None, // Notifications or bad messages
        };

        match req.method.as_str() {
            "initialize" => {
                let result = serde_json::json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {
                        "logging": {},
                        "prompts": { "listChanged": true },
                        "resources": { "subscribe": true, "listChanged": true },
                        "tools": { "listChanged": true }
                    },
                    "serverInfo": {
                        "name": "kak-claude",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                });
                let resp = JsonRpcResponse::success(req.id?, result);
                Some(serde_json::to_string(&resp).unwrap())
            }
            "notifications/initialized" => None, // No response
            "tools/list" => {
                let result = serde_json::json!({ "tools": tools::tool_list() });
                let resp = JsonRpcResponse::success(req.id?, result);
                Some(serde_json::to_string(&resp).unwrap())
            }
            "prompts/list" => {
                let result = serde_json::json!({ "prompts": [] });
                let resp = JsonRpcResponse::success(req.id?, result);
                Some(serde_json::to_string(&resp).unwrap())
            }
            "tools/call" => {
                self.handle_tool_call(req.id?, &req.params)
            }
            _ => {
                let resp = JsonRpcResponse::error(req.id?, METHOD_NOT_FOUND, "Method not found");
                Some(serde_json::to_string(&resp).unwrap())
            }
        }
    }

    fn handle_tool_call(&mut self, id: JsonRpcId, params: &serde_json::Value) -> Option<String> {
        let tool_name = params["name"].as_str().unwrap_or("");
        let args = &params["arguments"];

        let result = match tool_name {
            "getCurrentSelection" => {
                let json = self.state.current_selection().to_mcp_json();
                mcp_tool_response(json)
            }
            "getLatestSelection" => {
                let json = self.state.latest_selection().to_mcp_json();
                mcp_tool_response(json)
            }
            "getOpenEditors" => {
                let json = self.state.open_editors_json();
                mcp_tool_response(json)
            }
            "getWorkspaceFolders" => {
                let json = self.state.workspace_folders_json();
                mcp_tool_response(json)
            }
            "openFile" => {
                let path = args["filePath"].as_str().unwrap_or("");
                let start = args["startLine"].as_u64().map(|n| n as u32);
                let end = args["endLine"].as_u64().map(|n| n as u32);
                if let Some(start_line) = start {
                    let _ = self.kak.open_file_at(path, start_line, end);
                } else {
                    let _ = self.kak.open_file(path);
                }
                mcp_tool_response(serde_json::json!({"success": true}))
            }
            "openDiff" => {
                let old_path = args["old_file_path"].as_str().unwrap_or("");
                let new_contents = args["new_file_contents"].as_str().unwrap_or("");
                let tab_name = args["tab_name"].as_str().unwrap_or("diff");

                // Write new contents to temp file
                let tmp_dir = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
                let new_tmp = format!("{}/kak-claude-diff-{}", tmp_dir, uuid::Uuid::new_v4());
                let _ = std::fs::write(&new_tmp, new_contents);

                let req_id_str = match &id {
                    JsonRpcId::Number(n) => n.to_string(),
                    JsonRpcId::String(s) => s.clone(),
                };

                // Store pending and show diff
                let ws_token = self.active_ws_token.unwrap_or(Token(TOKEN_START));
                self.pending_diff.insert(req_id_str.clone(), (id.clone(), ws_token));
                let _ = self.kak.show_diff(old_path, &new_tmp, &req_id_str, 120);

                return None; // Deferred response
            }
            "checkDocumentDirty" => {
                let path = args["filePath"].as_str().unwrap_or("");
                let ws_token = self.active_ws_token.unwrap_or(Token(TOKEN_START));
                self.pending_dirty.insert(path.to_string(), (id.clone(), ws_token));
                let _ = self.kak.query_dirty(path);
                return None; // Deferred response
            }
            "saveDocument" => {
                let path = args["filePath"].as_str().unwrap_or("");
                let success = self.kak.save_buffer(path).is_ok();
                mcp_tool_response(serde_json::json!({"success": success}))
            }
            "closeAllDiffTabs" => {
                let _ = self.kak.close_diff_buffers();
                mcp_tool_response(serde_json::json!({"success": true}))
            }
            _ => {
                let resp = JsonRpcResponse::error(id, INVALID_PARAMS, &format!("Unknown tool: {tool_name}"));
                return Some(serde_json::to_string(&resp).unwrap());
            }
        };

        let resp = JsonRpcResponse::success(id, serde_json::json!({"content": result}));
        Some(serde_json::to_string(&resp).unwrap())
    }

    fn process_kak_message(&mut self, msg: KakMessage) {
        match msg {
            KakMessage::State { file, line, col, selection } => {
                self.state.update_selection(selection, file, line, col);
                self.broadcast_selection();
            }
            KakMessage::Buffers { list } => {
                self.state.update_buffers(&list);
            }
            KakMessage::Shutdown => {
                self.should_quit = true;
            }
            KakMessage::DirtyResponse { file, dirty } => {
                if let Some((rpc_id, ws_token)) = self.pending_dirty.remove(&file) {
                    let result = mcp_tool_response(serde_json::json!({
                        "success": true,
                        "isDirty": dirty
                    }));
                    let resp = JsonRpcResponse::success(rpc_id, serde_json::json!({"content": result}));
                    let text = serde_json::to_string(&resp).unwrap();
                    self.send_to_ws(ws_token, &text);
                }
            }
            KakMessage::DiffResponse { id, accepted } => {
                if let Some((rpc_id, ws_token)) = self.pending_diff.remove(&id) {
                    let result = if accepted {
                        mcp_tool_response(serde_json::json!({"success": true}))
                    } else {
                        mcp_tool_response(serde_json::json!({"success": false, "error": "User rejected changes"}))
                    };
                    let resp = JsonRpcResponse::success(rpc_id, serde_json::json!({"content": result}));
                    let text = serde_json::to_string(&resp).unwrap();
                    self.send_to_ws(ws_token, &text);
                }
            }
        }
    }

    fn broadcast_selection(&mut self) {
        let sel = self.state.current_selection();
        let notification = JsonRpcNotification::new("selection_changed", sel.to_mcp_json());
        let text = serde_json::to_string(&notification).unwrap();
        self.broadcast_ws(&text);
    }

    /// Send a message to a specific WebSocket client
    fn send_to_ws(&mut self, token: Token, text: &str) {
        if let Some(conn) = self.ws_connections.get_mut(&token) {
            if conn.is_connected() {
                conn.queue_message(text);
                conn.flush();
            }
        }
    }

    fn broadcast_ws(&mut self, text: &str) {
        for conn in self.ws_connections.values_mut() {
            if conn.is_connected() {
                conn.queue_message(text);
                conn.flush();
            }
        }
    }

    fn send_pings(&mut self) {
        let dead_tokens: Vec<Token> = self.ws_connections.iter_mut()
            .filter_map(|(token, conn)| {
                if !conn.ping() { Some(*token) } else { None }
            })
            .collect();
        for token in dead_tokens {
            self.ws_connections.remove(&token);
        }
    }

    fn cleanup(&self) {
        // Lock file cleaned up by Drop
        let tmpdir = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
        let session_dir = std::path::PathBuf::from(&tmpdir)
            .join("kak-claude")
            .join(self.kak.session_name());
        let _ = std::fs::remove_file(session_dir.join("sock"));
        let _ = std::fs::remove_file(session_dir.join("pid"));
        let _ = std::fs::remove_file(session_dir.join("port"));
        let _ = std::fs::remove_dir(&session_dir);
    }
}
```

Add `mod server;` to `src/main.rs`.

Note: The Unix socket event handling (`handle_unix_event`) needs Unix stream storage. This is addressed in the next step.

- [ ] **Step 2: Verify it compiles**

Run: `cd ~/Sites/rust/kak-claude && cargo build 2>&1`
Expected: Compiles (fix any API mismatches)

- [ ] **Step 3: Commit**

```bash
git add src/server.rs src/main.rs
git commit -m "feat: main server event loop with mio, WebSocket handling, and MCP dispatch"
```

---

### Task 11: Wire up main.rs start command

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Implement the start command**

Replace the `Command::Start` match arm:

```rust
Command::Start { session, client, cwd } => {
    // Create server (this binds sockets)
    let mut server = match server::Server::new(&session, &client, &cwd) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to start server: {e}");
            std::process::exit(1);
        }
    };

    // Print port to stdout (plugin reads this)
    println!("{}", server.port());

    // Daemonize (after printing port so parent gets it)
    // For now, just run in foreground for easier debugging
    // TODO: add daemonize support

    // Run the event loop
    if let Err(e) = server.run() {
        eprintln!("Server error: {e}");
        std::process::exit(1);
    }
}
```

- [ ] **Step 2: Verify the full binary compiles and runs**

Run: `cd ~/Sites/rust/kak-claude && cargo build 2>&1`
Expected: Compiles

Run: `cd ~/Sites/rust/kak-claude && timeout 2 cargo run -- start --session test --client main --cwd /tmp 2>&1 || true`
Expected: Prints a port number, then runs until timeout

- [ ] **Step 3: Commit**

```bash
git add src/main.rs
git commit -m "feat: wire up start command to server"
```

---

## Chunk 3: Kakoune Plugin + Integration

### Task 12: Kakoune plugin

**Files:**
- Create: `rc/claude.kak`

- [ ] **Step 1: Write the plugin**

```kak
# kak-claude: Claude Code IDE integration for Kakoune

declare-option -hidden str claude_pid
declare-option -hidden str claude_socket
declare-option -hidden str claude_ws_port

define-command claude -docstring 'Start Claude Code IDE integration' %{
  evaluate-commands %sh{
    tmpdir="${TMPDIR:-/tmp}"
    socket="$tmpdir/kak-claude/$kak_session/sock"
    pidfile="$tmpdir/kak-claude/$kak_session/pid"

    # Check if daemon already running
    if [ -f "$pidfile" ] && kill -0 "$(cat "$pidfile")" 2>/dev/null; then
      port=$(cat "$tmpdir/kak-claude/$kak_session/port")
      printf "set-option global claude_ws_port '%s'\n" "$port"
      printf "claude-open-terminal\n"
      exit
    fi

    # Start daemon — blocks until socket is ready (prints port to stdout)
    port=$(kak-claude start --session "$kak_session" --client "$kak_client" --cwd "$(pwd)")

    if [ -z "$port" ]; then
      printf "fail 'kak-claude: daemon failed to start'\n"
      exit
    fi

    printf "set-option global claude_socket '%s'\n" "$socket"
    printf "set-option global claude_ws_port '%s'\n" "$port"
    printf "claude-install-hooks\n"
    printf "claude-open-terminal\n"
  }
}

define-command -hidden claude-install-hooks %{
  hook -group claude global NormalIdle .* %{ claude-push-state }
  hook -group claude global InsertIdle .* %{ claude-push-state }
  hook -group claude global BufCreate  .* %{ claude-push-buffers }
  hook -group claude global BufClose   .* %{ claude-push-buffers }
  hook -group claude global KakEnd     .* %{ claude-shutdown }
}

define-command -hidden claude-push-state %{
  nop %sh{
    kak-claude send --session "$kak_session" state \
      --file "$kak_buffile" \
      --line "$kak_cursor_line" \
      --col "$kak_cursor_column" \
      --selection "$kak_selection" &
  }
}

define-command -hidden claude-push-buffers %{
  nop %sh{
    kak-claude send --session "$kak_session" buffers \
      --list "$kak_buflist" &
  }
}

define-command -hidden claude-shutdown %{
  nop %sh{
    kak-claude send --session "$kak_session" shutdown 2>/dev/null
  }
  remove-hooks global claude
}

define-command -hidden claude-open-terminal %{
  try %{
    terminal sh -c "CLAUDE_CODE_SSE_PORT=%opt{claude_ws_port} ENABLE_IDE_INTEGRATION=true claude"
  } catch %{
    echo -markup "{Error}kak-claude: Run claude manually with CLAUDE_CODE_SSE_PORT=%opt{claude_ws_port} ENABLE_IDE_INTEGRATION=true"
  }
}
```

- [ ] **Step 2: Commit**

```bash
git add rc/claude.kak
git commit -m "feat: Kakoune plugin with hooks, state push, and terminal launch"
```

---

### Task 13: Manual integration test

- [ ] **Step 1: Build release binary**

Run: `cd ~/Sites/rust/kak-claude && cargo build --release 2>&1`
Expected: Compiles successfully

- [ ] **Step 2: Test daemon starts and creates lock file**

```bash
cd ~/Sites/rust/kak-claude
timeout 3 ./target/release/kak-claude start --session test-session --client main --cwd /tmp &
sleep 1
# Check lock file exists
ls ~/.claude/ide/*.lock
# Check socket exists
ls ${TMPDIR:-/tmp}/kak-claude/test-session/sock
# Check port file
cat ${TMPDIR:-/tmp}/kak-claude/test-session/port
# Kill daemon
kill %1
```

Expected: Lock file, socket, and port file exist

- [ ] **Step 3: Test send command**

```bash
cd ~/Sites/rust/kak-claude
./target/release/kak-claude start --session test2 --client main --cwd /tmp &
sleep 1
./target/release/kak-claude send --session test2 state --file /tmp/test.rs --line 5 --col 3 --selection "hello"
./target/release/kak-claude send --session test2 buffers --list "file1.rs:file2.rs"
./target/release/kak-claude send --session test2 shutdown
wait
```

Expected: Daemon receives messages and shuts down cleanly

- [ ] **Step 4: Test WebSocket connection with wscat (if available)**

```bash
cd ~/Sites/rust/kak-claude
./target/release/kak-claude start --session test3 --client main --cwd /tmp &
sleep 1
PORT=$(cat ${TMPDIR:-/tmp}/kak-claude/test3/port)
AUTH=$(python3 -c "import json; print(json.load(open('$HOME/.claude/ide/$PORT.lock'))['authToken'])")
# Test with curl or wscat
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' | websocat ws://127.0.0.1:$PORT -H "x-claude-code-ide-authorization: $AUTH" --one-message
kill %1
```

Expected: Receives initialize response with protocolVersion

- [ ] **Step 5: Commit any fixes from integration testing**

```bash
git add -A
git commit -m "fix: integration test fixes"
```

---

### Task 14: Run all tests

- [ ] **Step 1: Run full test suite**

Run: `cd ~/Sites/rust/kak-claude && cargo test 2>&1`
Expected: All tests PASS

- [ ] **Step 2: Run clippy**

Run: `cd ~/Sites/rust/kak-claude && cargo clippy 2>&1`
Expected: No errors (warnings OK)

- [ ] **Step 3: Final commit**

```bash
git add -A
git commit -m "chore: clippy fixes and cleanup"
```

---

## Post-Implementation Notes

**What's not covered in this plan (intentional):**
- Daemonize support — left as foreground for easier debugging initially. Add `daemonize` crate integration after the core works.
- The `kak-claude start` command currently blocks in foreground. For production use, the plugin needs it to daemonize and the plugin reads the port from the port file instead of stdout.
- Debouncing selection updates (100ms) — can be added as a refinement after core works.
- The `show_diff` Kakoune escaping is a best-effort first pass — will need testing against a real Kakoune instance.

**Testing against real Claude CLI:**
After building, install the binary to PATH and source `rc/claude.kak` in your kakrc. Run `:claude` in Kakoune. Claude CLI should detect the IDE integration via the lock file and connect.
