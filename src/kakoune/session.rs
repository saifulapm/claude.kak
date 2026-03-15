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

    /// Create a lightweight clone for use in background threads
    pub fn clone_for_open(&self) -> Self {
        Self { session: self.session.clone(), client: self.client.clone() }
    }

    pub fn session_name(&self) -> &str {
        &self.session
    }

    pub fn client_name(&self) -> &str {
        &self.client
    }

    /// Send a command to Kakoune, targeting the stored client
    pub fn eval(&self, command: &str) -> std::io::Result<()> {
        let full_cmd = format!("evaluate-commands -client {} %{{{}}}", self.client, command);
        self.send_raw(&full_cmd)
    }

    /// Send a raw command to the Kakoune session (no client targeting)
    pub fn send_raw(&self, command: &str) -> std::io::Result<()> {
        let mut child = Command::new("kak")
            .arg("-p")
            .arg(&self.session)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(command.as_bytes())?;
            stdin.write_all(b"\n")?;
        }

        let output = child.wait_with_output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("/tmp/kak-claude-debug.log") {
                use std::io::Write;
                let _ = writeln!(f, "kak -p {} FAILED: {}", self.session, stderr);
            }
        }
        Ok(())
    }

    /// Open a file in the editor
    pub fn open_file(&self, path: &str) -> std::io::Result<()> {
        let escaped = path.replace('\'', "''");
        self.eval(&format!("edit! '{}'", escaped))
    }

    /// Open a file and select a line range
    pub fn open_file_at(&self, path: &str, start_line: u32, end_line: Option<u32>) -> std::io::Result<()> {
        let end = end_line.unwrap_or(start_line);
        self.eval(&format!(
            "edit! '{}'; execute-keys {}g{}G",
            path.replace('\'', "''"),
            start_line,
            end
        ))
    }

    /// Build the eval command string (exposed for testing)
    pub fn build_eval(&self, command: &str) -> String {
        format!("evaluate-commands -client {} %{{{}}}", self.client, command)
    }

    /// Show diff view in Kakoune
    /// Claude Code handles accept/reject in its own terminal — we just show the diff
    pub fn show_diff(&self, old_path: &str, new_path: &str, _request_id: &str, _width: u32) -> std::io::Result<()> {
        // Use a script file to avoid delimiter conflicts
        let tmp_dir = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
        let script = format!("{}/kak-claude-diff.sh", tmp_dir);
        std::fs::write(&script, format!(
            "#!/bin/sh\ndiff -u '{}' '{}' | delta --paging=never --file-style=omit --file-decoration-style=omit --hunk-header-style=omit --hunk-header-decoration-style=omit\n",
            old_path, new_path
        ))?;
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))?;

        let cmd = format!(
            "evaluate-commands -client {} %{{fifo -name '*claude-diff*' -scroll -- {}}}",
            self.client, script,
        );
        self.send_raw(&cmd)
    }

    /// Close all diff buffers
    pub fn close_diff_buffers(&self) -> std::io::Result<()> {
        self.send_raw("try %{evaluate-commands -buffer '*claude-diff*' delete-buffer}")
    }

    /// Save a buffer
    pub fn save_buffer(&self, path: &str) -> std::io::Result<()> {
        self.send_raw(&format!(
            "evaluate-commands -buffer '{}' write",
            path.replace('\'', "''")
        ))
    }

    /// Query if a buffer is dirty (response comes back via Unix socket)
    pub fn query_dirty(&self, path: &str) -> std::io::Result<()> {
        let escaped = path.replace('\'', "''");
        let cmd = format!(
            concat!(
                "evaluate-commands -buffer '{}' %<",
                "  nop %sh< kak-claude send --session \"$kak_session\" dirty-response --file '{}' --dirty \"$kak_modified\" >",
                ">"
            ),
            escaped, escaped,
        );
        self.send_raw(&cmd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_eval() {
        let kak = KakSession::new("test-session".into(), "main".into());
        let cmd = kak.build_eval("edit foo.rs");
        assert_eq!(cmd, "evaluate-commands -client main %{edit foo.rs}");
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
