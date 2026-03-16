use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

pub fn build_state_message(client: &str, file: &str, line: u32, col: u32, selection: &str, sel_desc: &str, sel_len: u32, error_count: u32, warning_count: u32) -> String {
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
        "warning_count": warning_count
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

pub fn build_diagnostics_response(file: &str, data: &str) -> String {
    serde_json::json!({
        "type": "diagnostics-response",
        "file": file,
        "data": data
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_state_message() {
        let msg = build_state_message("client0", "/tmp/f.rs", 10, 5, "hello", "10.5,10.10", 6, 0, 0);
        let parsed: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(parsed["type"], "state");
        assert_eq!(parsed["client"], "client0");
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
        let msg = build_state_message("", "/tmp/f.rs", 1, 1, "line with \"quotes\" and \nnewline", "1.1,2.5", 30, 0, 0);
        let parsed: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(parsed["selection"], "line with \"quotes\" and \nnewline");
    }
}
