# kak-claude

Claude Code IDE integration for [Kakoune](https://kakoune.org/).

A lightweight Rust daemon that bridges Kakoune and Claude Code via the MCP protocol over WebSocket — the same protocol used by the official VS Code and JetBrains extensions.

<!-- TODO: demo video -->

## Features

- Full MCP protocol support (selection tracking, file operations, diagnostics, diffs)
- Send selections and files to Claude as @mentions
- Real-time cursor/selection sync with Claude Code
- Kakoune-native multi-selection support
- Zero-delay shutdown, fire-and-forget daemon

## Requirements

- [Kakoune](https://kakoune.org/)
- [Claude Code CLI](https://docs.anthropic.com/en/docs/claude-code) (`claude`)
- Rust toolchain (to build)

## Installation

```sh
git clone https://github.com/rexa-dev/kak-claude.git
cd kak-claude
make install  # builds, codesigns (macOS), installs to ~/.local/bin
```

Add to your `kakrc`:

```kak
eval %sh{kak-claude init}
```

## Commands

| Command | Description |
|---------|-------------|
| `:claude-open` | Start daemon and open Claude terminal |
| `:claude-start` | Start daemon only (no terminal) |
| `:claude-stop` | Stop daemon |
| `:claude-send` | Send current selection to Claude as @mention |
| `:claude-add [path] [start] [end]` | Add file to Claude context |
| `:claude-status` | Show connection status |

## Quick Start

```
:claude-open          " start daemon + open Claude in terminal
                      " work in Kakoune, Claude sees your selections in real-time

:claude-send          " select code, send it to Claude as @mention

:claude-add           " add current file to Claude context
:claude-add src/      " add a directory
:claude-add foo.rs 10 50  " add lines 10-50

:claude-stop          " done, stop the daemon
```

## How It Works

```
Kakoune  ──hooks──>  kak-claude send  ──unix socket──>  daemon  ──WebSocket──>  Claude Code CLI
                     (fire-and-forget)                   (mio event loop)       (MCP protocol)
```

1. `eval %sh{kak-claude init}` defines commands and options
2. `:claude-open` starts the daemon and opens a terminal with Claude
3. Kakoune hooks (`NormalIdle`, `InsertIdle`, `FocusIn`, etc.) push editor state to the daemon
4. The daemon serves MCP tools over WebSocket — Claude can open files, check diagnostics, show diffs
5. `:claude-send` and `:claude-add` broadcast `at_mentioned` notifications to Claude

