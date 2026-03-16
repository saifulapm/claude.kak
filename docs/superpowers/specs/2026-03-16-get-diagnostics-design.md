# getDiagnostics MCP Tool

## Overview

Add `getDiagnostics` MCP tool to expose kakoune-lsp diagnostics to Claude Code. On-demand query via `kak -p`, same async pattern as `checkDocumentDirty`. Also push error/warning counts with each state update.

## getDiagnostics Tool

### Input Schema

```json
{
  "uri": "string (optional, file path — if omitted, returns current buffer diagnostics)"
}
```

### Output Format

```json
{
  "uri": "file:///path/to/file.rs",
  "diagnostics": [
    {
      "range": {
        "start": { "line": 0, "character": 5 },
        "end": { "line": 0, "character": 10 }
      },
      "severity": 1,
      "message": "unused variable"
    }
  ]
}
```

Severity: 1=Error, 2=Warning, 3=Info, 4=Hint (LSP standard).

### Query Flow

1. Claude calls `getDiagnostics` with optional `uri`
2. Daemon stores pending request, sends `kak -p` to Kakoune
3. Kakoune reads `$kak_opt_lsp_inline_diagnostics` (ranges + severity) and `$kak_opt_lsp_inlay_diagnostics` (messages)
4. Kakoune runs `kak-claude send --session <s> diagnostics-response --file <path> --data <json>`
5. Daemon matches pending request, sends MCP response

### Kakoune Query Command

The `kak -p` command reads diagnostic options and constructs JSON:

```sh
kak-claude send --session "$kak_session" diagnostics-response \
  --file "$kak_buffile" \
  --data "$diagnostics_json"
```

The shell block parses `$kak_opt_lsp_inline_diagnostics` (range-specs: `line.col,line.col|DiagnosticError`) and `$kak_opt_lsp_inlay_diagnostics` (line-specs with messages) to build a JSON array of diagnostics.

## State Push Enhancement

Add to `claude-push-state`:
```
--error-count "$kak_opt_lsp_diagnostic_error_count"
--warning-count "$kak_opt_lsp_diagnostic_warning_count"
```

Include in `getOpenEditors` response per buffer: `"diagnosticCounts": { "errors": N, "warnings": N }`.

## New Message Types

### CLI: `diagnostics-response` subcommand
```
kak-claude send --session <s> diagnostics-response --file <path> --data <json>
```

### Socket message
```json
{
  "type": "diagnostics-response",
  "file": "/path/to/file.rs",
  "data": "[{\"range\":...,\"severity\":1,\"message\":\"...\"}]"
}
```

## Files Changed

- `src/mcp/tools.rs` — add getDiagnostics tool schema
- `src/server.rs` — getDiagnostics handler (deferred), diagnostics-response processing
- `src/kakoune/session.rs` — `query_diagnostics()` method
- `src/kakoune/socket.rs` — DiagnosticsResponse message type
- `src/kakoune/state.rs` — error/warning counts in state, include in open_editors_json
- `src/client.rs` — diagnostics-response message builder
- `src/main.rs` — DiagnosticsResponse send subcommand, error-count/warning-count in State
- `rc/claude.kak` — push diagnostic counts, query_diagnostics kak command
