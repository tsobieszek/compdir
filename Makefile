BIN_NAME := compdir
PREFIX ?= $(HOME)/bin
ZFUNC_DIR ?= $(HOME)/.zfunc
COMPLETION_SOURCE := completions/_compdir
TARGET_DIR := target/release
BIN_PATH := $(TARGET_DIR)/$(BIN_NAME)

.PHONY: all build install clean

all: build

build:
	cargo build --release

install: build
	mkdir -p "$(PREFIX)" "$(ZFUNC_DIR)"
	install -m 755 "$(BIN_PATH)" "$(PREFIX)/$(BIN_NAME)"
	install -m 644 "$(COMPLETION_SOURCE)" "$(ZFUNC_DIR)/_$(BIN_NAME)"

clean:
	cargo clean
