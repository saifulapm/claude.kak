# kak-claude: Claude Code IDE integration for Kakoune

declare-option -hidden str claude_pid
declare-option -hidden str claude_socket
declare-option -hidden str claude_ws_port

define-command claude -docstring 'Start Claude Code IDE integration' %{
  evaluate-commands %sh{
    tmpdir="${TMPDIR:-/tmp}"
    socket="$tmpdir/kak-claude/$kak_session/sock"
    pidfile="$tmpdir/kak-claude/$kak_session/pid"

    # Check if daemon already running
    if [ -f "$pidfile" ] && kill -0 "$(cat "$pidfile")" 2>/dev/null; then
      port=$(cat "$tmpdir/kak-claude/$kak_session/port")
      printf "set-option global claude_ws_port '%s'\n" "$port"
      printf "claude-open-terminal\n"
      exit
    fi

    # Start daemon in background (no fork — macOS kills forked processes from app contexts)
    kak-claude start --session "$kak_session" --client "$kak_client" --cwd "$(pwd)" \
      </dev/null >/dev/null 2>&1 &

    # Wait for daemon to be ready (socket file appears)
    for i in $(seq 1 30); do
      [ -S "$socket" ] && break
      sleep 0.1
    done

    if [ ! -S "$socket" ]; then
      printf "fail 'kak-claude: daemon failed to start'\n"
      exit
    fi

    port=$(cat "$tmpdir/kak-claude/$kak_session/port")

    printf "set-option global claude_socket '%s'\n" "$socket"
    printf "set-option global claude_ws_port '%s'\n" "$port"
    printf "claude-install-hooks\n"
    printf "claude-open-terminal\n"
  }
}

define-command -hidden claude-install-hooks %{
  hook -group claude global NormalIdle .* %{ claude-push-state }
  hook -group claude global InsertIdle .* %{ claude-push-state }
  hook -group claude global BufCreate  .* %{ claude-push-buffers }
  hook -group claude global BufClose   .* %{ claude-push-buffers }
  hook -group claude global KakEnd     .* %{ claude-shutdown }
}

define-command -hidden claude-push-state %{
  nop %sh{
    # selection_length=1 means just cursor position (no real selection in Kakoune)
    kak-claude send --session "$kak_session" state \
      --file "$kak_buffile" \
      --line "$kak_cursor_line" \
      --col "$kak_cursor_column" \
      --selection "$kak_selection" \
      --sel-desc "$kak_selection_desc" \
      --sel-len "$kak_selection_length" &
  }
}

define-command -hidden claude-push-buffers %{
  nop %sh{
    # kak_buflist is space-separated in shell context
    kak-claude send --session "$kak_session" buffers \
      --list "$kak_quoted_buflist" &
  }
}

define-command -hidden claude-shutdown %{
  nop %sh{
    tmpdir="${TMPDIR:-/tmp}"
    pidfile="$tmpdir/kak-claude/$kak_session/pid"
    # Send graceful shutdown
    kak-claude send --session "$kak_session" shutdown 2>/dev/null
    # Wait briefly, then force kill if still running
    sleep 0.5
    if [ -f "$pidfile" ]; then
      pid=$(cat "$pidfile")
      kill "$pid" 2>/dev/null
      # Clean up lock files
      rm -f ~/.claude/ide/*.lock 2>/dev/null
      rm -rf "$tmpdir/kak-claude/$kak_session" 2>/dev/null
    fi
  }
  remove-hooks global claude
}

define-command -hidden claude-open-terminal %{
  try %{
    terminal sh -c "CLAUDE_CODE_SSE_PORT=%opt{claude_ws_port} ENABLE_IDE_INTEGRATION=true CLAUDE_CODE_AUTO_CONNECT_IDE=true KAKOUNE_SESSION=%val{session} KAKOUNE_CLIENT=%val{client} claude --ide --append-system-prompt 'You are connected to Kakoune editor via IDE integration. ALWAYS use the openFile MCP tool to open files in the editor instead of shell commands. Use saveDocument to save files. The user can see files you open in their Kakoune editor.'"
  } catch %{
    echo -markup "{Error}kak-claude: Run claude manually with CLAUDE_CODE_SSE_PORT=%opt{claude_ws_port} ENABLE_IDE_INTEGRATION=true"
  }
}
