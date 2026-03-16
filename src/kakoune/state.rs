use std::path::Path;

#[derive(Debug, Clone)]
pub struct Selection {
    pub text: String,
    pub file_path: String,
    pub line: u32,      // 1-based (Kakoune native)
    pub col: u32,       // 1-based (Kakoune native)
    #[allow(dead_code)]
    pub sel_desc: String, // Kakoune selection_desc "anchor.col,cursor.col"
    pub sel_len: u32,     // selection length in codepoints (1 = cursor only)
}

impl Selection {
    pub fn empty(file_path: &str, line: u32, col: u32) -> Self {
        Self { text: String::new(), file_path: file_path.into(), line, col, sel_desc: String::new(), sel_len: 0 }
    }

    /// In Kakoune, cursor always has 1-char selection. Real selection is len > 1.
    pub fn is_cursor_only(&self) -> bool {
        self.sel_len <= 1
    }

    /// Convert to MCP JSON with 0-based positions
    pub fn to_mcp_json(&self) -> serde_json::Value {
        let line_0 = if self.line > 0 { self.line - 1 } else { 0 };
        let col_0 = if self.col > 0 { self.col - 1 } else { 0 };
        let is_empty = self.is_cursor_only();

        // Estimate end position from text content
        let (end_line, end_col) = if is_empty {
            (line_0, col_0)
        } else {
            let lines: Vec<&str> = self.text.split('\n').collect();
            let end_l = line_0 + (lines.len() as u32) - 1;
            let end_c = if lines.len() == 1 {
                col_0 + lines[0].chars().count() as u32
            } else {
                lines.last().unwrap().chars().count() as u32
            };
            (end_l, end_c)
        };

        // When cursor only (sel_len <= 1), report empty text to Claude
        let reported_text = if is_empty { "" } else { &self.text };

        serde_json::json!({
            "text": reported_text,
            "filePath": self.file_path,
            "fileUrl": format!("file://{}", self.file_path),
            "selection": {
                "start": { "line": line_0, "character": col_0 },
                "end": { "line": end_line, "character": end_col },
                "isEmpty": is_empty
            }
        })
    }

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
    error_count: u32,
    warning_count: u32,
    pub line_count: u32,
    pub is_dirty: bool,
}

impl EditorState {
    pub fn new(cwd: String) -> Self {
        Self {
            current: Selection::empty("", 0, 0),
            latest: Selection::empty("", 0, 0),
            buffers: Vec::new(),
            cwd,
            error_count: 0,
            warning_count: 0,
            line_count: 0,
            is_dirty: false,
        }
    }

    pub fn update_selection(&mut self, text: String, file: String, line: u32, col: u32, sel_desc: String, sel_len: u32, error_count: u32, warning_count: u32, line_count: u32, modified: bool) {
        self.error_count = error_count;
        self.warning_count = warning_count;
        self.line_count = line_count;
        self.is_dirty = modified;
        self.current = Selection { text: text.clone(), file_path: file.clone(), line, col, sel_desc, sel_len };
        // Only update latest if there's a real selection (not just cursor)
        if sel_len > 1 {
            self.latest = self.current.clone();
        }
    }

    pub fn current_selection(&self) -> &Selection {
        &self.current
    }

    pub fn latest_selection(&self) -> &Selection {
        &self.latest
    }

    pub fn update_buffers(&mut self, buflist: &str) {
        // Kakoune's $kak_quoted_buflist is shell-quoted space-separated
        // e.g: 'file1.rs' 'file 2.rs' '*debug*'
        // Simple parsing: split on ' ' boundaries respecting single quotes
        self.buffers = parse_kak_quoted_list(buflist)
            .into_iter()
            .filter(|s| !s.is_empty())
            .map(|s| BufferInfo { path: s, is_active: false })
            .collect();
        // Mark first buffer as active (simplification)
        if let Some(first) = self.buffers.first_mut() {
            first.is_active = true;
        }
    }

    pub fn has_buffer(&self, path: &str) -> bool {
        self.buffers.iter().any(|b| b.path == path || format!("{}/{}", self.cwd, b.path) == path)
    }

    pub fn count_diff_buffers(&self) -> usize {
        self.buffers.iter().filter(|b| b.path.contains("claude-diff")).count()
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
        let tabs: Vec<serde_json::Value> = self.buffers.iter()
            .filter(|b| !b.path.starts_with('*') && !b.path.starts_with("debug"))
            .map(|b| {
            let full_path = if b.path.starts_with('/') {
                b.path.clone()
            } else {
                format!("{}/{}", self.cwd, b.path)
            };
            let file_name = Path::new(&full_path)
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
}

/// Parse Kakoune's shell-quoted list format
/// Input: "'file1.rs' 'file 2.rs' '*debug*'" or "file1.rs file2.rs"
fn parse_kak_quoted_list(input: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\'' if !in_quote => {
                in_quote = true;
            }
            '\'' if in_quote => {
                // Check for escaped quote ''
                if chars.peek() == Some(&'\'') {
                    current.push('\'');
                    chars.next();
                } else {
                    in_quote = false;
                }
            }
            ' ' if !in_quote => {
                if !current.is_empty() {
                    result.push(std::mem::take(&mut current));
                }
            }
            _ => {
                current.push(c);
            }
        }
    }
    if !current.is_empty() {
        result.push(current);
    }
    result
}

pub fn guess_language(path: &str) -> &'static str {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_update_selection() {
        let mut state = EditorState::new("/tmp/project".into());
        state.update_selection("hello world".into(), "/tmp/file.rs".into(), 10, 5, "10.5,10.16".into(), 11, 0, 0, 0, false);
        let sel = state.current_selection();
        assert_eq!(sel.text, "hello world");
        assert_eq!(sel.file_path, "/tmp/file.rs");
        assert_eq!(sel.line, 10);
        assert_eq!(sel.col, 5);
    }

    #[test]
    fn test_latest_selection_preserved() {
        let mut state = EditorState::new("/tmp/project".into());
        // Make a real selection (sel_len > 1)
        state.update_selection("selected text".into(), "/tmp/a.rs".into(), 5, 1, "5.1,5.13".into(), 13, 0, 0, 0, false);
        // Move cursor (sel_len = 1) — latest should be preserved
        state.update_selection("x".into(), "/tmp/a.rs".into(), 10, 1, "10.1,10.1".into(), 1, 0, 0, 0, false);
        let current = state.current_selection();
        assert_eq!(current.text, "x");
        let latest = state.latest_selection();
        assert_eq!(latest.text, "selected text");
    }

    #[test]
    fn test_parse_buflist() {
        let mut state = EditorState::new("/tmp".into());
        state.update_buffers("'file1.rs' 'file2.rs' '*debug*'");
        assert_eq!(state.buffers.len(), 3);
        assert_eq!(state.buffers[0].path, "file1.rs");
    }

    #[test]
    fn test_selection_to_mcp_json() {
        let mut state = EditorState::new("/tmp".into());
        state.update_selection("hi".into(), "/tmp/f.rs".into(), 3, 7, "3.7,3.9".into(), 2, 0, 0, 0, false);
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
}
