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

    # Start daemon (forks to background, returns immediately)
    kak-claude start --session "$kak_session" --client "$kak_client" --cwd "$(pwd)"

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
    kak-claude send --session "$kak_session" state \
      --file "$kak_buffile" \
      --line "$kak_cursor_line" \
      --col "$kak_cursor_column" \
      --selection "$kak_selection" &
  }
}

define-command -hidden claude-push-buffers %{
  nop %sh{
    kak-claude send --session "$kak_session" buffers \
      --list "$kak_buflist" &
  }
}

define-command -hidden claude-shutdown %{
  nop %sh{
    kak-claude send --session "$kak_session" shutdown 2>/dev/null
  }
  remove-hooks global claude
}

define-command -hidden claude-open-terminal %{
  try %{
    terminal sh -c "CLAUDE_CODE_SSE_PORT=%opt{claude_ws_port} ENABLE_IDE_INTEGRATION=true claude"
  } catch %{
    echo -markup "{Error}kak-claude: Run claude manually with CLAUDE_CODE_SSE_PORT=%opt{claude_ws_port} ENABLE_IDE_INTEGRATION=true"
  }
}
