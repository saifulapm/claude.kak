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
    #[allow(dead_code)]
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

        // Track active WS token
        self.active_ws_token = Some(token);

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
                self.pending_diff.insert(req_id_str.clone(), (id, ws_token));
                let _ = self.kak.show_diff(old_path, &new_tmp, &req_id_str, 120);

                return None; // Deferred response
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
