PREFIX ?= $(HOME)/.local
UNAME := $(shell uname -s)

.PHONY: build install clean

build:
	cargo build --release
ifeq ($(UNAME),Darwin)
	codesign --sign - --force target/release/kak-claude
endif

install: build
	install -d $(PREFIX)/bin
	install target/release/kak-claude $(PREFIX)/bin/
ifeq ($(UNAME),Darwin)
	codesign --sign - --force $(PREFIX)/bin/kak-claude
endif

clean:
	cargo clean
