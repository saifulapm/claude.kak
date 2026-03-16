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

/// Run diff -u and parse hunk headers to get added line ranges in the new file.
/// Returns (start_line, end_line) 1-based inclusive ranges of added lines.
fn compute_changed_ranges(old_path: &str, new_path: &str) -> Vec<(u32, u32)> {
    let output = std::process::Command::new("diff")
        .arg("-u")
        .arg(old_path)
        .arg(new_path)
        .output();

    let output = match output {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };

    let diff_text = String::from_utf8_lossy(&output.stdout);
    let mut ranges: Vec<(u32, u32)> = Vec::new();

    // Parse diff -u output: track + lines with their actual line numbers
    let mut new_line_num: u32 = 0;
    let mut in_hunk = false;
    let mut added_start: Option<u32> = None;

    for line in diff_text.lines() {
        if line.starts_with("@@") {
            // Parse @@ -X,Y +Z,W @@ — Z is the start line in new file
            if let Some(plus_pos) = line.find('+') {
                let rest = &line[plus_pos + 1..];
                let num_str = rest.split(|c: char| !c.is_ascii_digit()).next().unwrap_or("0");
                new_line_num = num_str.parse().unwrap_or(0);
                if new_line_num > 0 {
                    new_line_num -= 1; // will be incremented on first line
                }
            }
            in_hunk = true;
            // Flush any pending range
            if let Some(start) = added_start.take() {
                ranges.push((start, new_line_num.saturating_sub(1).max(start)));
            }
            continue;
        }

        if !in_hunk {
            continue;
        }

        if line.starts_with('+') {
            new_line_num += 1;
            if added_start.is_none() {
                added_start = Some(new_line_num);
            }
        } else {
            // Context line or deletion — flush pending added range
            if let Some(start) = added_start.take() {
                ranges.push((start, new_line_num));
            }
            if !line.starts_with('-') {
                new_line_num += 1; // context line
            }
            // deleted lines don't increment new_line_num
        }
    }

    // Flush final range
    if let Some(start) = added_start {
        ranges.push((start, new_line_num));
    }

    ranges
}

struct PendingDiff {
    rpc_id: JsonRpcId,
    ws_token: Token,
    file_path: String,
    new_contents: String,
    old_tmp_path: String,  // temp file with old content for diff
    new_tmp_path: String,  // temp file with new content for diff
    tab_name: String,
}

pub struct Server {
    poll: Poll,
    unix_listener: UnixListener,
    tcp_listener: TcpListener,
    state: EditorState,
    kak: KakSession,
    #[allow(dead_code)]
    lockfile: LockFile,
    ws_connections: HashMap<Token, WsConnection>,
    unix_streams: HashMap<Token, mio::net::UnixStream>,
    unix_buffers: HashMap<Token, Vec<u8>>,
    next_token: usize,
    pending_dirty: HashMap<String, (JsonRpcId, Token)>,
    pending_diagnostics: HashMap<String, (JsonRpcId, Token)>,
    pending_diff: HashMap<String, PendingDiff>,
    last_diff_file_path: Option<String>,
    /// Token of the most recently active WebSocket client (for targeting responses)
    active_ws_token: Option<Token>,
    last_ping: Instant,
    last_selection_broadcast: Instant,
    pending_selection: bool,
    should_quit: bool,
}

impl Server {
    #[allow(dead_code)]
    pub fn new(session: &str, client: &str, cwd: &str) -> io::Result<Self> {
        let tcp_addr: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();
        let std_listener = std::net::TcpListener::bind(tcp_addr)?;
        std_listener.set_nonblocking(true)?;
        Self::with_tcp_listener(session, client, cwd, std_listener)
    }

    /// Create server with a pre-bound TCP listener (used when forking)
    pub fn with_tcp_listener(session: &str, client: &str, cwd: &str, std_tcp: std::net::TcpListener) -> io::Result<Self> {
        let poll = Poll::new()?;

        // Unix socket
        let tmpdir = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
        let session_dir = std::path::PathBuf::from(&tmpdir)
            .join("kak-claude")
            .join(session);
        std::fs::create_dir_all(&session_dir)?;
        let sock_path = session_dir.join("sock");
        let _ = std::fs::remove_file(&sock_path);
        let mut unix_listener = UnixListener::bind(&sock_path)?;
        poll.registry().register(&mut unix_listener, UNIX_LISTENER, Interest::READABLE)?;

        // Convert std TcpListener to mio TcpListener
        let mut tcp_listener = TcpListener::from_std(std_tcp);
        let port = tcp_listener.local_addr()?.port();
        poll.registry().register(&mut tcp_listener, TCP_LISTENER, Interest::READABLE)?;

        // Write port file
        let port_file = session_dir.join("port");
        std::fs::write(&port_file, port.to_string())?;

        // Write PID file (child PID after fork)
        let pid_file = session_dir.join("pid");
        std::fs::write(&pid_file, std::process::id().to_string())?;

        // Lock file (with child PID)
        let lockfile = LockFile::create(std::process::id(), port, &[cwd])
            .map_err(io::Error::other)?;

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
            pending_diagnostics: HashMap::new(),
            pending_diff: HashMap::new(),
            last_diff_file_path: None,
            active_ws_token: None,
            last_ping: Instant::now(),
            last_selection_broadcast: Instant::now(),
            pending_selection: false,
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
        let debounce = Duration::from_millis(100);

        while !self.should_quit {
            // Dynamic timeout: shorter when we have a pending selection to flush
            let timeout = if self.pending_selection {
                let elapsed = self.last_selection_broadcast.elapsed();
                if elapsed >= debounce {
                    Some(Duration::ZERO)
                } else {
                    Some(debounce - elapsed)
                }
            } else {
                Some(Duration::from_secs(5))
            };

            self.poll.poll(&mut events, timeout)?;

            // Check for signal-requested shutdown
            if crate::SHUTDOWN_REQUESTED.load(std::sync::atomic::Ordering::SeqCst) {
                self.should_quit = true;
            }

            // Flush pending debounced selection
            if self.pending_selection && self.last_selection_broadcast.elapsed() >= debounce {
                self.broadcast_selection();
                self.last_selection_broadcast = Instant::now();
                self.pending_selection = false;
            }

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

        // Only track as active if it sent real messages
        if !messages.is_empty() {
            self.active_ws_token = Some(token);
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
        // Collect messages first, then process (avoids borrow issues)
        let mut kak_messages = Vec::new();
        if let Some(buf) = self.unix_buffers.get_mut(&token) {
            while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = buf.drain(..=pos).collect();
                if let Ok(text) = std::str::from_utf8(&line[..line.len()-1]) {
                    if let Ok(msg) = KakMessage::parse(text) {
                        kak_messages.push(msg);
                    }
                }
            }
        }

        for msg in kak_messages {
            self.process_kak_message(msg);
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
            "resources/list" => {
                let result = serde_json::json!({ "resources": [] });
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
                let _tab_name = args["tab_name"].as_str().unwrap_or("diff");

                let tmp_dir = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());

                // Always copy old content to a temp file — the original file will be
                // overwritten by Claude before close_tab computes the diff ranges
                let old_tmp = format!("{}/kak-claude-old-{}", tmp_dir, uuid::Uuid::new_v4());
                if !old_path.is_empty() && std::path::Path::new(old_path).exists() {
                    let _ = std::fs::copy(old_path, &old_tmp);
                } else {
                    let _ = std::fs::write(&old_tmp, "");
                }
                let old_actual = old_tmp;

                // Write new contents to temp file
                let new_tmp = format!("{}/kak-claude-diff-{}", tmp_dir, uuid::Uuid::new_v4());
                let _ = std::fs::write(&new_tmp, new_contents);

                let req_id_str = match &id {
                    JsonRpcId::Number(n) => n.to_string(),
                    JsonRpcId::String(s) => s.clone(),
                };

                // Show diff in Kakoune
                let new_file_path = args["new_file_path"].as_str().unwrap_or("").to_string();
                let tab_name = args["tab_name"].as_str().unwrap_or("").to_string();
                self.last_diff_file_path = Some(new_file_path.clone());
                let _ = self.kak.show_diff(&old_actual, &new_tmp, "", 120);

                // Read old file content for computing changed lines later
                let old_contents = if !old_path.is_empty() && std::path::Path::new(old_path).exists() {
                    std::fs::read_to_string(old_path).unwrap_or_default()
                } else {
                    String::new()
                };

                // DEFERRED: Claude's terminal shows accept/reject
                let ws_token = self.active_ws_token.unwrap_or(Token(TOKEN_START));
                self.pending_diff.insert(req_id_str, PendingDiff {
                    rpc_id: id,
                    ws_token,
                    file_path: new_file_path,
                    new_contents: new_contents.to_string(),
                    old_tmp_path: old_actual.clone(),
                    new_tmp_path: new_tmp.clone(),
                    tab_name,
                });
                return None;
            }
            "checkDocumentDirty" => {
                let path = args["filePath"].as_str().unwrap_or("");
                let ws_token = self.active_ws_token.unwrap_or(Token(TOKEN_START));
                self.pending_dirty.insert(path.to_string(), (id, ws_token));
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
            "getDiagnostics" => {
                let uri = args.get("uri").and_then(|v| v.as_str()).unwrap_or("");
                let path = if uri.starts_with("file://") { &uri[7..] } else if !uri.is_empty() { uri } else { "" };
                let query_path = if path.is_empty() {
                    self.state.current_selection().file_path.clone()
                } else {
                    path.to_string()
                };
                let ws_token = self.active_ws_token.unwrap_or(Token(TOKEN_START));
                self.pending_diagnostics.insert(query_path.clone(), (id, ws_token));
                let _ = self.kak.query_diagnostics(&query_path);
                return None;
            }
            "close_tab" => {
                // Resolve deferred openDiff with FILE_SAVED + contents
                let tab_name = args["tab_name"].as_str().unwrap_or("");
                let mut changed_lines: Vec<(u32, u32)> = Vec::new();
                let mut resolved_file_path = None;

                // Find the pending diff matching this tab_name
                let matching_key = self.pending_diff.iter()
                    .find(|(_, pd)| pd.tab_name == tab_name)
                    .map(|(k, _)| k.clone());

                if let Some(key) = matching_key {
                    if let Some(pd) = self.pending_diff.remove(&key) {
                        changed_lines = compute_changed_ranges(&pd.old_tmp_path, &pd.new_tmp_path);
                        resolved_file_path = Some(pd.file_path.clone());

                        // Clean up temp files
                        let _ = std::fs::remove_file(&pd.old_tmp_path);
                        let _ = std::fs::remove_file(&pd.new_tmp_path);

                        let result = serde_json::json!([
                            { "type": "text", "text": "FILE_SAVED" },
                            { "type": "text", "text": pd.new_contents }
                        ]);
                        let resp = JsonRpcResponse::success(pd.rpc_id, serde_json::json!({"content": result}));
                        let text = serde_json::to_string(&resp).unwrap();
                        self.send_to_ws(pd.ws_token, &text);
                    }
                }

                // Close diff buffer, open file and select changed lines — all in one kak -p call
                let _ = self.kak.close_diff_buffers();
                let file_path = resolved_file_path.or_else(|| self.last_diff_file_path.take());
                if let Some(path) = file_path {
                    let kak = self.kak.clone_for_open();
                    let lines = changed_lines;
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_millis(200));
                        let escaped = path.replace('\'', "''");
                        if lines.is_empty() {
                            let _ = kak.eval(&format!("edit! '{}'", escaped));
                        } else {
                            let selections: Vec<String> = lines.iter()
                                .map(|(start, end)| format!("{}.1,{}.999999", start, end))
                                .collect();
                            let sel_str = selections.join(" ");
                            // Single command: open file then select changed lines
                            let cmd = format!("edit! '{}'; select {}", escaped, sel_str);
                            let _ = kak.eval(&cmd);
                        }
                    });
                }
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
            KakMessage::State { client, file, line, col, selection, sel_desc, sel_len, error_count, warning_count, line_count, modified } => {
                // Update active client for multi-window support
                self.kak.set_client(&client);
                // Skip scratch buffers (e.g. *claude-diff*, *debug*)
                if file.starts_with('*') || file.is_empty() {
                    return;
                }
                self.state.update_selection(selection, file, line, col, sel_desc, sel_len, error_count, warning_count, line_count, modified);
                let debounce = Duration::from_millis(100);
                if self.last_selection_broadcast.elapsed() >= debounce {
                    self.broadcast_selection();
                    self.last_selection_broadcast = Instant::now();
                    self.pending_selection = false;
                } else {
                    self.pending_selection = true;
                }
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
            KakMessage::DiagnosticsResponse { file, data } => {
                if let Some((rpc_id, ws_token)) = self.pending_diagnostics.remove(&file) {
                    let diagnostics: serde_json::Value = serde_json::from_str(&data).unwrap_or(serde_json::json!([]));
                    let result = mcp_tool_response(serde_json::json!({
                        "uri": format!("file://{}", file),
                        "diagnostics": diagnostics
                    }));
                    let resp = JsonRpcResponse::success(rpc_id, serde_json::json!({"content": result}));
                    let text = serde_json::to_string(&resp).unwrap();
                    self.send_to_ws(ws_token, &text);
                }
            }
            KakMessage::DiffResponse { id, accepted } => {
                if let Some(pd) = self.pending_diff.remove(&id) {
                    // Clean up temp files
                    let _ = std::fs::remove_file(&pd.old_tmp_path);
                    let _ = std::fs::remove_file(&pd.new_tmp_path);

                    let result = if accepted {
                        serde_json::json!([
                            { "type": "text", "text": "FILE_SAVED" },
                            { "type": "text", "text": pd.new_contents }
                        ])
                    } else {
                        serde_json::json!([
                            { "type": "text", "text": "DIFF_REJECTED" },
                            { "type": "text", "text": pd.tab_name }
                        ])
                    };
                    let resp = JsonRpcResponse::success(pd.rpc_id, serde_json::json!({"content": result}));
                    let text = serde_json::to_string(&resp).unwrap();
                    self.send_to_ws(pd.ws_token, &text);
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

        // Clean up any remaining diff/old temp files
        if let Ok(entries) = std::fs::read_dir(&tmpdir) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    if name.starts_with("kak-claude-diff-") || name.starts_with("kak-claude-old-") {
                        let _ = std::fs::remove_file(entry.path());
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_changed_ranges_addition() {
        let dir = tempfile::TempDir::new().unwrap();
        let old = dir.path().join("old");
        let new = dir.path().join("new");
        std::fs::write(&old, "line1\nline2\nline3\n").unwrap();
        std::fs::write(&new, "line1\nline2\nadded\nline3\n").unwrap();
        let ranges = compute_changed_ranges(old.to_str().unwrap(), new.to_str().unwrap());
        assert_eq!(ranges, vec![(3, 3)]);
    }

    #[test]
    fn test_compute_changed_ranges_modification() {
        let dir = tempfile::TempDir::new().unwrap();
        let old = dir.path().join("old");
        let new = dir.path().join("new");
        std::fs::write(&old, "line1\nline2\nline3\n").unwrap();
        std::fs::write(&new, "line1\nchanged\nline3\n").unwrap();
        let ranges = compute_changed_ranges(old.to_str().unwrap(), new.to_str().unwrap());
        assert_eq!(ranges, vec![(2, 2)]);
    }

    #[test]
    fn test_compute_changed_ranges_new_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let old = dir.path().join("old");
        let new = dir.path().join("new");
        std::fs::write(&old, "").unwrap();
        std::fs::write(&new, "line1\nline2\n").unwrap();
        let ranges = compute_changed_ranges(old.to_str().unwrap(), new.to_str().unwrap());
        assert_eq!(ranges, vec![(1, 2)]);
    }

    #[test]
    fn test_compute_changed_ranges_no_changes() {
        let dir = tempfile::TempDir::new().unwrap();
        let old = dir.path().join("old");
        let new = dir.path().join("new");
        std::fs::write(&old, "same\n").unwrap();
        std::fs::write(&new, "same\n").unwrap();
        let ranges = compute_changed_ranges(old.to_str().unwrap(), new.to_str().unwrap());
        assert!(ranges.is_empty());
    }

    #[test]
    fn test_compute_changed_ranges_multiple_blocks() {
        let dir = tempfile::TempDir::new().unwrap();
        let old = dir.path().join("old");
        let new = dir.path().join("new");
        std::fs::write(&old, "a\nb\nc\nd\ne\n").unwrap();
        std::fs::write(&new, "a\nX\nc\nY\nZ\ne\n").unwrap();
        let ranges = compute_changed_ranges(old.to_str().unwrap(), new.to_str().unwrap());
        assert_eq!(ranges, vec![(2, 2), (4, 5)]);
    }
}
