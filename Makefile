# Common tasks. `make install` builds Solilog.app, installs it and puts the
# `solilog` command on the PATH (a link into the app, so both stay in sync).

APP_DIR ?= /Applications
BIN_DIR ?= $(HOME)/.local/bin

VERSION := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
APP     := target/release/Solilog.app
MACOS   := packaging/macos

.PHONY: help run test test-core clippy check release bundle install uninstall icon clean

help: ## List the targets
	@grep -E '^[a-z-]+:.*## ' $(MAKEFILE_LIST) | awk -F ':.*## ' '{printf "  %-10s %s\n", $$1, $$2}'

run: ## Run the desktop app (debug build)
	cargo run

test: ## Run all tests
	cargo test --workspace

test-core: ## Run the core tests only (fast, no gpui)
	cargo test -p solilog-core

clippy: ## Lint, warnings as errors
	cargo clippy --workspace --all-targets -- -D warnings

check: test clippy ## Tests and lints

release: ## Release build of the binary
	cargo build --release -p solilog

bundle: release ## Build target/release/Solilog.app
	rm -rf $(APP)
	mkdir -p $(APP)/Contents/MacOS $(APP)/Contents/Resources
	cp target/release/solilog $(APP)/Contents/MacOS/solilog
	cp $(MACOS)/Solilog.icns $(APP)/Contents/Resources/Solilog.icns
	sed 's/@VERSION@/$(VERSION)/g' $(MACOS)/Info.plist > $(APP)/Contents/Info.plist
	codesign --force --sign - $(APP)  # ad-hoc, so macOS treats it as one app

install: bundle ## Install the app in APP_DIR and link the CLI into BIN_DIR
	-osascript -e 'if application "Solilog" is running then quit app "Solilog"'
	rm -rf "$(APP_DIR)/Solilog.app"
	ditto $(APP) "$(APP_DIR)/Solilog.app"
	mkdir -p "$(BIN_DIR)"
	ln -sf "$(APP_DIR)/Solilog.app/Contents/MacOS/solilog" "$(BIN_DIR)/solilog"
	@case ":$$PATH:" in *":$(BIN_DIR):"*) ;; *) echo "note: $(BIN_DIR) is not on your PATH";; esac

uninstall: ## Remove the app and the CLI link (the database is kept)
	rm -rf "$(APP_DIR)/Solilog.app"
	rm -f "$(BIN_DIR)/solilog"

icon: ## Regenerate the .icns from icon.svg (needs rsvg-convert)
	$(MACOS)/icon.sh

clean: ## Remove build output
	cargo clean
