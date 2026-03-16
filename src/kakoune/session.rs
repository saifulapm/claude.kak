use std::io::Write;
use std::process::{Command, Stdio};

/// Escape a string for safe use in single-quoted shell arguments.
fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

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
    /// Fire-and-forget: spawns kak -p without waiting for it to exit.
    /// Child processes are reaped via SIGCHLD SIG_IGN set at daemon startup.
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
        // Drop stdin (closes pipe), don't wait — fire and forget
        // Child process will be reaped by SIGCHLD
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
            "#!/bin/sh\ndiff -u {} {} | delta --paging=never --file-style=omit --file-decoration-style=omit --hunk-header-style=omit --hunk-header-decoration-style=omit\n",
            shell_escape(old_path), shell_escape(new_path)
        ))?;
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))?;

        let cmd = format!(
            "evaluate-commands -client {} %{{fifo -name '*claude-diff*' -scroll -- {}}}",
            self.client, script,
        );
        let result = self.send_raw(&cmd);
        // Script has been read by kak -p; clean it up
        let _ = std::fs::remove_file(&script);
        result
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

    /// Query diagnostics for a buffer (response comes back via Unix socket)
    pub fn query_diagnostics(&self, path: &str) -> std::io::Result<()> {
        let kak_escaped = path.replace('\'', "''");
        let shell_path = shell_escape(path);

        // Write a script that parses lsp_inline_diagnostics (ranges+severity)
        // and lsp_inlay_diagnostics (messages) into JSON
        let tmp_dir = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
        let script = format!("{}/kak-claude-diag-query.sh", tmp_dir);
        std::fs::write(&script, r#"#!/bin/sh
# Parse kakoune-lsp diagnostics into LSP JSON
# Args: $1=session $2=file $3=inline_diagnostics_file $4=inlay_diagnostics_file
SESSION="$1"
FILE="$2"
INLINE_FILE="$3"
INLAY_FILE="$4"

# Read inlay diagnostics to extract messages per line
# Format: "line|spaces symbols {face}message" (one per line in the file)
# We store line -> message mapping
declare -A msgs 2>/dev/null || true  # bash associative array, fallback for sh

# Parse inlay file: each line is a quoted entry like "19|     ■ {InlayDiagnosticError}msg"
if [ -f "$INLAY_FILE" ]; then
  while IFS= read -r raw_entry; do
    # Strip quotes
    entry="${raw_entry#\"}"
    entry="${entry%\"}"
    # Extract line number (before |)
    line="${entry%%|*}"
    # Extract message: everything after the last }
    rest="${entry#*\}}"
    if [ -n "$rest" ] && [ -n "$line" ]; then
      # Store message for this line (escape quotes for JSON)
      escaped_msg=$(printf '%s' "$rest" | sed 's/\\/\\\\/g; s/"/\\"/g; s/	/ /g')
      eval "msg_$line=\"\$escaped_msg\"" 2>/dev/null
    fi
  done < "$INLAY_FILE"
fi

# Parse inline diagnostics (from file, space-separated: timestamp entry entry...)
diags=""
if [ -f "$INLINE_FILE" ]; then
  inline_data=$(cat "$INLINE_FILE")
  # Skip first word (timestamp)
  set -- $inline_data
  shift
  for entry in "$@"; do
    range="${entry%|*}"
    face="${entry#*|}"
    start="${range%,*}"
    end="${range#*,}"
    sl="${start%.*}"
    sc="${start#*.}"
    el="${end%.*}"
    ec="${end#*.}"
    case "$face" in
      DiagnosticError) sev=1 ;;
      DiagnosticWarning) sev=2 ;;
      DiagnosticInfo) sev=3 ;;
      DiagnosticHint) sev=4 ;;
      *) sev=1 ;;
    esac
    # Get message for this line
    msg=""
    eval "msg=\$msg_$sl" 2>/dev/null
    # Convert 1-based to 0-based
    sl=$((sl - 1))
    sc=$((sc - 1))
    el=$((el - 1))
    ec=$((ec - 1))
    if [ -n "$diags" ]; then diags="$diags,"; fi
    diags="$diags{\"range\":{\"start\":{\"line\":$sl,\"character\":$sc},\"end\":{\"line\":$el,\"character\":$ec}},\"severity\":$sev,\"message\":\"$msg\"}"
  done
fi
kak-claude send --session "$SESSION" diagnostics-response --file "$FILE" --data "[$diags]"
"#)?;
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))?;

        // Kakoune command: dump both diagnostic options to temp files, run script
        let cmd = format!(
            concat!(
                "evaluate-commands -buffer '{}' %<",
                "  nop %sh<",
                "    inline_f=$(mktemp)",
                "    inlay_f=$(mktemp)",
                "    printf '%s' \"$kak_opt_lsp_inline_diagnostics\" > \"$inline_f\"",
                "    printf '%s\\n' $kak_quoted_opt_lsp_inlay_diagnostics > \"$inlay_f\"",
                "    {} \"$kak_session\" {} \"$inline_f\" \"$inlay_f\"",
                "    rm -f \"$inline_f\" \"$inlay_f\"",
                "  >",
                ">"
            ),
            kak_escaped, script, shell_path,
        );
        self.send_raw(&cmd)
    }

    /// Query if a buffer is dirty (response comes back via Unix socket)
    pub fn query_dirty(&self, path: &str) -> std::io::Result<()> {
        let kak_escaped = path.replace('\'', "''");
        let shell_escaped = shell_escape(path);
        let cmd = format!(
            concat!(
                "evaluate-commands -buffer '{}' %<",
                "  nop %sh< kak-claude send --session \"$kak_session\" dirty-response --file {} --dirty \"$kak_modified\" >",
                ">"
            ),
            kak_escaped, shell_escaped,
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
