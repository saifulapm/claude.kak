use serde::Deserialize;

#[derive(Debug)]
pub enum KakMessage {
    State { client: String, file: String, line: u32, col: u32, selection: String, sel_desc: String, sel_len: u32, error_count: u32, warning_count: u32, line_count: u32, modified: bool },
    Buffers { list: String },
    Shutdown,
    DirtyResponse { file: String, dirty: bool },
    DiffResponse { id: String, accepted: bool },
    DiagnosticsResponse { file: String, data: String },
    AtMention { file: String, line_start: Option<i64>, line_end: Option<i64> },
}

#[derive(Deserialize)]
struct RawMessage {
    #[serde(rename = "type")]
    msg_type: String,
    #[serde(default)]
    client: String,
    #[serde(default)]
    file: String,
    #[serde(default)]
    line: u32,
    #[serde(default)]
    col: u32,
    #[serde(default)]
    selection: String,
    #[serde(default)]
    sel_desc: String,
    #[serde(default)]
    sel_len: u32,
    #[serde(default)]
    error_count: u32,
    #[serde(default)]
    warning_count: u32,
    #[serde(default)]
    line_count: u32,
    #[serde(default)]
    modified: bool,
    #[serde(default)]
    list: String,
    #[serde(default)]
    dirty: bool,
    #[serde(default)]
    id: String,
    #[serde(default)]
    accepted: bool,
    #[serde(default)]
    data: String,
    #[serde(default)]
    line_start: Option<i64>,
    #[serde(default)]
    line_end: Option<i64>,
}

impl KakMessage {
    pub fn parse(input: &str) -> Result<Self, String> {
        let raw: RawMessage = serde_json::from_str(input)
            .map_err(|e| format!("Invalid JSON: {e}"))?;

        match raw.msg_type.as_str() {
            "state" => Ok(KakMessage::State {
                client: raw.client,
                file: raw.file,
                line: raw.line,
                col: raw.col,
                selection: raw.selection,
                sel_desc: raw.sel_desc,
                sel_len: raw.sel_len,
                error_count: raw.error_count,
                warning_count: raw.warning_count,
                line_count: raw.line_count,
                modified: raw.modified,
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
            "diagnostics-response" => Ok(KakMessage::DiagnosticsResponse {
                file: raw.file,
                data: raw.data,
            }),
            "at-mention" => Ok(KakMessage::AtMention {
                file: raw.file,
                line_start: raw.line_start,
                line_end: raw.line_end,
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
        let msg = r#"{"type":"state","client":"client0","file":"/tmp/f.rs","line":10,"col":5,"selection":"hello","sel_desc":"10.5,10.10","sel_len":6}"#;
        let parsed = KakMessage::parse(msg).unwrap();
        match parsed {
            KakMessage::State { client, file, line, col, selection, sel_desc, sel_len, .. } => {
                assert_eq!(client, "client0");
                assert_eq!(file, "/tmp/f.rs");
                assert_eq!(line, 10);
                assert_eq!(col, 5);
                assert_eq!(selection, "hello");
                assert_eq!(sel_len, 6);
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
    fn test_parse_at_mention() {
        let msg = r#"{"type":"at-mention","file":"src/main.rs","line_start":0,"line_end":10}"#;
        let parsed = KakMessage::parse(msg).unwrap();
        match parsed {
            KakMessage::AtMention { file, line_start, line_end } => {
                assert_eq!(file, "src/main.rs");
                assert_eq!(line_start, Some(0));
                assert_eq!(line_end, Some(10));
            }
            _ => panic!("Expected AtMention"),
        }
    }

    #[test]
    fn test_parse_at_mention_no_lines() {
        let msg = r#"{"type":"at-mention","file":"lib.rs"}"#;
        let parsed = KakMessage::parse(msg).unwrap();
        match parsed {
            KakMessage::AtMention { file, line_start, line_end } => {
                assert_eq!(file, "lib.rs");
                assert!(line_start.is_none());
                assert!(line_end.is_none());
            }
            _ => panic!("Expected AtMention"),
        }
    }

    #[test]
    fn test_parse_invalid_json() {
        assert!(KakMessage::parse("not json").is_err());
    }
}
