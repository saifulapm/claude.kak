# High-Value Kakoune Commands

## Goal

Add `claude-send`, `claude-add`, `claude-status`, and `claude-stop` commands to kak-claude, matching claudecode.nvim's `ClaudeCodeSend`, `ClaudeCodeAdd`, `ClaudeCodeStatus`, and `ClaudeCodeStop` functionality.

## Reference

- claudecode.nvim `at_mentioned` notification: `lua/claudecode/init.lua`
- MCP notification format: JSON-RPC 2.0 without `id` field

---

## 1. claude-send — Send selection as @mention

### Behavior
- Fail with `fail 'claude-send: no selection (cursor only)'` if `$kak_selection_length == 1`
- Extract file path from `$kak_buffile` and line range from `$kak_selection_desc`
- Send `at-mention` message to daemon via unix socket
- Daemon broadcasts `at_mentioned` notification to all WebSocket clients

### Kakoune command
```kak
define-command claude-send -docstring 'Send current selection to Claude as @mention' %{
  evaluate-commands %sh{
    if [ "$kak_selection_length" -le 1 ]; then
      printf "fail 'claude-send: no selection (cursor only)'\n"
      exit
    fi
    # Parse selection_desc "anchor_line.anchor_col,cursor_line.cursor_col"
    # Extract start and end lines, convert to 0-indexed
    anchor_line="${kak_selection_desc%%.*}"
    cursor_part="${kak_selection_desc##*,}"
    cursor_line="${cursor_part%%.*}"
    if [ "$anchor_line" -le "$cursor_line" ]; then
      start_line=$((anchor_line - 1))
      end_line=$((cursor_line - 1))
    else
      start_line=$((cursor_line - 1))
      end_line=$((anchor_line - 1))
    fi
    # Make path relative to cwd if possible
    file="$kak_buffile"
    cwd="$(pwd)"
    case "$file" in "$cwd/"*) file="${file#$cwd/}" ;; esac
    kak-claude send --session "$kak_session" at-mention \
      --file "$file" --line-start "$start_line" --line-end "$end_line" &
  }
}
```

### WebSocket notification (broadcast by daemon)
```json
{
  "jsonrpc": "2.0",
  "method": "at_mentioned",
  "params": {
    "filePath": "src/main.rs",
    "lineStart": 0,
    "lineEnd": 10
  }
}
```

- `filePath`: relative to cwd when possible, absolute otherwise
- `lineStart`, `lineEnd`: 0-indexed (matching nvim)

---

## 2. claude-add — Add file/range to context

### Behavior
- `claude-add` (no args): add current buffer file, no line range
- `claude-add <path>`: add specific file (validate exists)
- `claude-add <path> <start> <end>`: add file with 1-indexed line range (converted to 0-indexed)
- For directories: add trailing `/` to path
- Same `at_mentioned` notification as `claude-send`

### Kakoune command
```kak
define-command claude-add -params ..3 \
  -docstring 'Add file to Claude context. Usage: claude-add [path] [start-line] [end-line]' %{
  evaluate-commands %sh{
    if [ $# -eq 0 ]; then
      file="$kak_buffile"
    else
      file="$1"
      # Expand relative paths
      if [ "${file#/}" = "$file" ]; then
        file="$(pwd)/$file"
      fi
      if [ ! -e "$file" ]; then
        printf "fail 'claude-add: file not found: %s'\n" "$1"
        exit
      fi
    fi
    # Make path relative to cwd if possible
    cwd="$(pwd)"
    case "$file" in "$cwd/"*) file="${file#$cwd/}" ;; esac
    # Add trailing slash for directories
    if [ -d "$file" ] || [ -d "$(pwd)/$file" ]; then
      case "$file" in */) ;; *) file="$file/" ;; esac
    fi
    line_start=""
    line_end=""
    if [ $# -ge 2 ]; then
      line_start=$(($2 - 1))
    fi
    if [ $# -ge 3 ]; then
      line_end=$(($3 - 1))
    fi
    kak-claude send --session "$kak_session" at-mention \
      --file "$file" \
      ${line_start:+--line-start "$line_start"} \
      ${line_end:+--line-end "$line_end"} &
  }
}
```

File completion for the path argument:
```kak
complete-command claude-add file
```

---

## 3. claude-status — Show connection status

### Behavior
- Check PID file exists and process is alive
- Show port if running
- Pure kak script, no daemon interaction

### Kakoune command
```kak
define-command claude-status -docstring 'Show Claude integration status' %{
  evaluate-commands %sh{
    tmpdir="${TMPDIR:-/tmp}"
    pidfile="$tmpdir/kak-claude/$kak_session/pid"
    if [ -f "$pidfile" ] && kill -0 "$(cat "$pidfile")" 2>/dev/null; then
      port=$(cat "$tmpdir/kak-claude/$kak_session/port" 2>/dev/null)
      printf "echo 'claude: running on port %s'\n" "$port"
    else
      printf "echo 'claude: not running'\n"
    fi
  }
}
```

---

## 4. claude-stop — Stop daemon

### Behavior
- Send shutdown (fire-and-forget)
- Remove hooks
- User-facing version of `claude-shutdown`

### Kakoune command
```kak
define-command claude-stop -docstring 'Stop Claude Code integration' %{
  remove-hooks global claude
  nop %sh{
    kak-claude send --session "$kak_session" shutdown 2>/dev/null &
  }
}
```

---

## 5. Daemon-side: AtMention message handling

### New SendMessage variant in main.rs
```rust
AtMention {
    #[arg(long)]
    file: String,
    #[arg(long)]
    line_start: Option<i64>,
    #[arg(long)]
    line_end: Option<i64>,
}
```

### New message builder in client.rs
```rust
pub fn build_at_mention_message(file: &str, line_start: Option<i64>, line_end: Option<i64>) -> String {
    serde_json::json!({
        "type": "at-mention",
        "file": file,
        "line_start": line_start,
        "line_end": line_end
    }).to_string()
}
```

### New KakMessage variant in socket.rs
```rust
AtMention { file: String, line_start: Option<i64>, line_end: Option<i64> }
```

### Server handler in server.rs
On receiving `KakMessage::AtMention`, broadcast to all WebSocket clients:
```rust
KakMessage::AtMention { file, line_start, line_end } => {
    let mut params = serde_json::json!({ "filePath": file });
    if let Some(ls) = line_start {
        params["lineStart"] = serde_json::json!(ls);
    }
    if let Some(le) = line_end {
        params["lineEnd"] = serde_json::json!(le);
    }
    let notification = JsonRpcNotification::new("at_mentioned", params);
    let text = serde_json::to_string(&notification).unwrap();
    self.broadcast_ws(&text);
}
```

---

## Files Changed

| File | Changes |
|------|---------|
| `rc/claude.kak` | Add `claude-send`, `claude-add`, `claude-status`, `claude-stop` commands |
| `src/main.rs` | Add `AtMention` variant to `SendMessage` enum |
| `src/client.rs` | Add `build_at_mention_message()` |
| `src/kakoune/socket.rs` | Add `AtMention` to `KakMessage`, `RawMessage` fields |
| `src/server.rs` | Handle `AtMention` — broadcast `at_mentioned` notification |

## Unresolved Questions

None.
