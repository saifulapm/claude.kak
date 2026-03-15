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
            .stderr(Stdio::null())
            .spawn()?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(command.as_bytes())?;
            stdin.write_all(b"\n")?;
        }

        child.wait()?;
        Ok(())
    }

    /// Open a file in the editor
    pub fn open_file(&self, path: &str) -> std::io::Result<()> {
        let escaped = path.replace('\'', "''");
        // edit works for both existing and new files in Kakoune
        self.eval(&format!("edit -force '{}'", escaped))
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
        format!("evaluate-commands -client {} %{{{}}}", self.client, command)
    }

    /// Show diff in a fifo buffer and prompt for accept/reject
    pub fn show_diff(&self, old_path: &str, new_path: &str, request_id: &str, width: u32) -> std::io::Result<()> {
        let escaped_old = old_path.replace('\'', "''");
        let escaped_new = new_path.replace('\'', "''");
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
