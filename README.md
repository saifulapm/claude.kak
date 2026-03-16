# Kak Claude

Claude Code IDE integration for [Kakoune](https://kakoune.org/).

A daemon that bridges Kakoune and Claude Code via MCP protocol.

## Installation

```sh
git clone https://github.com/rexa-dev/kak-claude.git
cd kak-claude
cargo build --release
codesign --sign - --force target/release/kak-claude  # macOS
install -d ~/.local/bin && install target/release/kak-claude ~/.local/bin/
```

Add to your `kakrc`:
```kak
source /path/to/kak-claude/rc/claude.kak
```

## Usage

In Kakoune: `:claude`
In Claude Code: `/ide`

## Author

Rexa

## License

[MIT](LICENSE)
