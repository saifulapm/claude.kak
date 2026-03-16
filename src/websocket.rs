use mio::net::TcpStream;
use std::io;
use tungstenite::handshake::server::ServerHandshake;
use tungstenite::handshake::MidHandshake;
use tungstenite::{HandshakeError, Message, WebSocket};

/// Callback struct for WebSocket handshake auth validation
pub struct AuthCallback {
    token: String,
}

impl tungstenite::handshake::server::Callback for AuthCallback {
    fn on_request(
        self,
        request: &tungstenite::handshake::server::Request,
        response: tungstenite::handshake::server::Response,
    ) -> Result<tungstenite::handshake::server::Response, tungstenite::handshake::server::ErrorResponse>
    {
        let auth = request.headers().get("x-claude-code-ide-authorization");
        match auth {
            Some(val) if val.to_str().unwrap_or("") == self.token => Ok(response),
            _ => {
                let resp = tungstenite::http::Response::builder()
                    .status(403)
                    .body(Some("Unauthorized".into()))
                    .unwrap();
                Err(resp)
            }
        }
    }
}

pub enum WsState {
    /// Waiting for WebSocket handshake
    Pending(TcpStream),
    /// Mid-handshake (non-blocking retry needed)
    Handshaking(MidHandshake<ServerHandshake<TcpStream, AuthCallback>>),
    /// Fully connected WebSocket
    Connected(WebSocket<TcpStream>),
    /// Connection closed
    Closed,
}

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

    /// Attempt or continue WebSocket handshake.
    /// Returns Ok(true) when handshake is complete, Ok(false) when still in progress.
    pub fn try_handshake(&mut self, auth_token: &str) -> Result<bool, String> {
        let state = std::mem::replace(&mut self.state, WsState::Closed);

        match state {
            WsState::Pending(stream) => {
                let callback = AuthCallback { token: auth_token.to_string() };
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
                Ok(Message::Pong(_)) => {
                    self.last_pong = std::time::Instant::now();
                    Ok(None)
                }
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
        self.write_queue.push_back(Message::text(text));
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

    pub fn is_alive(&self, timeout: std::time::Duration) -> bool {
        self.last_pong.elapsed() < timeout
    }

    pub fn reset_pong_timer(&mut self) {
        self.last_pong = std::time::Instant::now();
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
