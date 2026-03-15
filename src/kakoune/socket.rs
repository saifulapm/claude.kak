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
