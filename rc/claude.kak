# kak-claude: Claude Code IDE integration for Kakoune

declare-option -hidden str claude_pid
declare-option -hidden str claude_socket
declare-option -hidden str claude_ws_port

define-command claude-start -docstring 'Start Claude Code daemon (server only, no terminal)' %{
  evaluate-commands %sh{
    tmpdir="${TMPDIR:-/tmp}"
    socket="$tmpdir/kak-claude/$kak_session/sock"
    pidfile="$tmpdir/kak-claude/$kak_session/pid"

    # Check if daemon already running
    if [ -f "$pidfile" ] && kill -0 "$(cat "$pidfile")" 2>/dev/null; then
      port=$(cat "$tmpdir/kak-claude/$kak_session/port")
      printf "set-option global claude_ws_port '%s'\n" "$port"
      printf "echo 'claude: already running on port %s'\n" "$port"
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
    printf "echo 'claude: started on port %s'\n" "$port"
  }
}

define-command claude-open -docstring 'Start Claude Code and open terminal' %{
  claude-start
  claude-open-terminal
}

define-command -hidden claude-install-hooks %{
  hook -group claude global NormalIdle .* %{ claude-push-state }
  hook -group claude global InsertIdle .* %{ claude-push-state }
  hook -group claude global FocusIn .* %{ claude-push-state }
  hook -group claude global WinDisplay .* %{ claude-push-state; claude-push-buffers }
  hook -group claude global BufCreate  .* %{ claude-push-buffers }
  hook -group claude global BufClose   .* %{ claude-push-buffers }
  hook -group claude global KakEnd     .* %{ claude-shutdown }
}

define-command -hidden claude-push-state %{
  nop %sh{
    # selection_length=1 means just cursor position (no real selection in Kakoune)
    # Pass selection via stdin to avoid ARG_MAX limits on large selections
    printf '%s' "$kak_selection" | kak-claude send --session "$kak_session" state \
      --client "$kak_client" \
      --file "$kak_buffile" \
      --line "$kak_cursor_line" \
      --col "$kak_cursor_column" \
      --selection "" \
      --sel-desc "$kak_selection_desc" \
      --sel-len "$kak_selection_length" \
      --selection-stdin \
      --error-count "${kak_opt_lsp_diagnostic_error_count:-0}" \
      --warning-count "${kak_opt_lsp_diagnostic_warning_count:-0}" \
      --line-count "${kak_buf_line_count:-0}" \
      --modified "${kak_modified:-false}" &
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
  remove-hooks global claude
  nop %sh{
    # Fire-and-forget: daemon cleans up its own files (sock, pid, port, lock)
    kak-claude send --session "$kak_session" shutdown 2>/dev/null &
  }
}

define-command claude-send -docstring 'Send current selection to Claude as @mention' %{
  evaluate-commands %sh{
    if [ "$kak_selection_length" -le 1 ]; then
      printf "fail 'claude-send: no selection (cursor only)'\n"
      exit
    fi
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
    file="$kak_buffile"
    cwd="$(pwd)"
    case "$file" in "$cwd/"*) file="${file#$cwd/}" ;; esac
    kak-claude send --session "$kak_session" at-mention \
      --file "$file" --line-start "$start_line" --line-end "$end_line" &
    printf "echo 'Sent to Claude: %s:%d-%d'\n" "$file" "$((start_line + 1))" "$((end_line + 1))"
  }
}

define-command claude-add -params ..3 \
  -docstring 'Add file to Claude context. Usage: claude-add [path] [start-line] [end-line]' %{
  evaluate-commands %sh{
    if [ $# -eq 0 ]; then
      file="$kak_buffile"
      if [ -z "$file" ] || [ "${file#\*}" != "$file" ]; then
        printf "fail 'claude-add: current buffer has no file'\n"
        exit
      fi
    else
      file="$1"
      if [ "${file#/}" = "$file" ]; then
        file="$(pwd)/$file"
      fi
      if [ ! -e "$file" ]; then
        printf "fail 'claude-add: file not found: %s'\n" "$1"
        exit
      fi
    fi
    cwd="$(pwd)"
    case "$file" in "$cwd/"*) file="${file#$cwd/}" ;; esac
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
    printf "echo 'Added to Claude: %s'\n" "$file"
  }
}
complete-command claude-add file

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

define-command claude-stop -docstring 'Stop Claude Code integration' %{
  remove-hooks global claude
  nop %sh{
    kak-claude send --session "$kak_session" shutdown 2>/dev/null &
  }
}

define-command -hidden claude-open-terminal %{
  try %{
    terminal -- env CLAUDE_CODE_SSE_PORT=%opt{claude_ws_port} ENABLE_IDE_INTEGRATION=true claude
  } catch %{
    echo -markup "{Error}kak-claude: set CLAUDE_CODE_SSE_PORT=%opt{claude_ws_port} ENABLE_IDE_INTEGRATION=true and run claude"
  }
}
