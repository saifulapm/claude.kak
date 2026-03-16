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

    /// Update the active client name (e.g. when user switches Kakoune windows)
    pub fn set_client(&mut self, client: &str) {
        if !client.is_empty() {
            self.client = client.to_string();
        }
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

    /// Open a file and select a precise range using Kakoune's select command
    /// All positions are 1-based (Kakoune native)
    pub fn open_file_select_range(&self, path: &str, start_line: u32, start_col: u32, end_line: u32, end_col: u32) -> std::io::Result<()> {
        let escaped = path.replace('\'', "''");
        self.eval(&format!(
            "edit! '{}' {}; select {}.{},{}.{}",
            escaped, start_line, start_line, start_col, end_line, end_col
        ))
    }

    /// Open a file, select range, then extend to end of line
    pub fn open_file_select_to_eol(&self, path: &str, start_line: u32, start_col: u32, end_line: u32) -> std::io::Result<()> {
        let escaped = path.replace('\'', "''");
        self.eval(&format!(
            "edit! '{}' {}; select {}.{},{}.999999; execute-keys <a-l>",
            escaped, start_line, start_line, start_col, end_line,
        ))
    }

    /// Build the eval command string (exposed for testing)
    #[cfg(test)]
    pub fn build_eval(&self, command: &str) -> String {
        format!("evaluate-commands -client {} %{{{}}}", self.client, command)
    }

    /// Show diff view in Kakoune
    /// Claude Code handles accept/reject in its own terminal — we just show the diff
    pub fn show_diff(&self, old_path: &str, new_path: &str, _request_id: &str, _width: u32) -> std::io::Result<()> {
        let tmp_dir = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
        let fifo_path = format!("{}/kak-claude-fifo-{}", tmp_dir, uuid::Uuid::new_v4());

        // Create named pipe from Rust
        let c_path = std::ffi::CString::new(fifo_path.clone()).unwrap();
        let ret = unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) };
        if ret != 0 {
            return Err(std::io::Error::last_os_error());
        }

        // Start diff writer in background thread — blocks on write until Kakoune reads
        let old = old_path.to_string();
        let new = new_path.to_string();
        let fifo = fifo_path.clone();
        std::thread::spawn(move || {
            let output = std::process::Command::new("sh")
                .arg("-c")
                .arg(format!(
                    "diff -u {} {} | delta --paging=never --file-style=omit --file-decoration-style=omit --hunk-header-style=omit --hunk-header-decoration-style=omit",
                    shell_escape(&old), shell_escape(&new)
                ))
                .output();
            if let Ok(out) = output {
                // Write to fifo — this blocks until Kakoune opens the reader
                let _ = std::fs::write(&fifo, out.stdout);
            }
            let _ = std::fs::remove_file(&fifo);
        });

        // Tell Kakoune to open the fifo buffer (this unblocks the writer thread)
        self.eval(&format!("edit -fifo {} -scroll '*claude-diff*'", fifo_path))
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

        // Write a script that parses lsp diagnostics into LSP JSON
        // Uses temp files per line to store messages (avoids shell eval escaping issues)
        let tmp_dir = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
        let script = format!("{}/kak-claude-diag-query.sh", tmp_dir);
        std::fs::write(&script, r#"#!/bin/sh
# Args: $1=session $2=file $3=inline_diagnostics_raw $4=inlay_diagnostics_raw
SESSION="$1"
FILE="$2"
INLINE="$3"
INLAY="$4"

# Create temp dir for message files (one per line number)
msgdir=$(mktemp -d)

# Parse inlay diagnostics using perl to split on " NUMBER|" boundaries
echo "$INLAY" | perl -pe 's/ (\d+\|)/\n$1/g' | while IFS= read -r entry; do
  [ -z "$entry" ] && continue
  line="${entry%%|*}"
  case "$line" in *[!0-9]*) continue ;; esac
  # Message is after the second }
  rest="${entry#*\}}"
  rest="${rest#*\}}"
  msg=$(printf '%s' "$rest" | sed 's/^ *//')
  printf '%s' "$msg" > "$msgdir/$line"
done

# Parse inline diagnostics and build JSON
set -- $INLINE
shift  # skip timestamp
diags=""
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
  msg=""
  if [ -f "$msgdir/$sl" ]; then
    msg=$(cat "$msgdir/$sl" | sed 's/\\/\\\\/g; s/"/\\"/g')
  fi
  sl=$((sl - 1)); sc=$((sc - 1)); el=$((el - 1)); ec=$((ec - 1))
  if [ -n "$diags" ]; then diags="$diags,"; fi
  diags="$diags{\"range\":{\"start\":{\"line\":$sl,\"character\":$sc},\"end\":{\"line\":$el,\"character\":$ec}},\"severity\":$sev,\"message\":\"$msg\"}"
done
rm -rf "$msgdir"
kak-claude send --session "$SESSION" diagnostics-response --file "$FILE" --data "[$diags]"
"#)?;
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))?;

        // Kakoune command: pass both raw option values to script
        let cmd = format!(
            concat!(
                "evaluate-commands -buffer '{}' %<",
                "  nop %sh< {} \"$kak_session\" {} \"$kak_opt_lsp_inline_diagnostics\" \"$kak_opt_lsp_inlay_diagnostics\" >",
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
    }

    #[test]
    fn test_open_file_escapes_quotes() {
        let kak = KakSession::new("s".into(), "c".into());
        let cmd = kak.build_eval(&format!("edit '{}'", "it's".replace('\'', "''")));
        assert!(cmd.contains("it''s"));
    }
}
