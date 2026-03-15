PREFIX ?= $(HOME)/.local

.PHONY: build install clean

build:
	cargo build --release
	codesign --sign - --force target/release/kak-claude

install: build
	install -d $(PREFIX)/bin
	install target/release/kak-claude $(PREFIX)/bin/
	codesign --sign - --force $(PREFIX)/bin/kak-claude

clean:
	cargo clean
