# Common tasks. `make install` installs the app and puts the `solilog`
# command on the PATH:
# - macOS: builds Solilog.app, installs it in APP_DIR and links the command
#   into the app, so both stay in sync.
# - Linux: installs the binary in BIN_DIR, plus a desktop entry and an icon
#   in DATA_DIR, so the app shows up in the desktop's app launcher.
# Both then install or update the agent skill (`solilog skill install`), as
# anyone with only the binary would; it fails harmlessly when no coding
# agent is installed.

UNAME    ?= $(shell uname -s)
APP_ID   := dev.solilog.Solilog
BIN_DIR  ?= $(HOME)/.local/bin
APP_DIR  ?= /Applications
DATA_DIR ?= $(HOME)/.local/share

VERSION := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
APP     := target/release/Solilog.app
MACOS   := packaging/macos
LINUX   := packaging/linux
DESKTOP := $(DATA_DIR)/applications/$(APP_ID).desktop
ICON    := $(DATA_DIR)/icons/hicolor/scalable/apps/$(APP_ID).svg

.PHONY: help run demo screenshot test test-core clippy check release bundle install uninstall icon clean

help: ## List the targets
	@grep -E '^[a-z-]+:.*## ' $(MAKEFILE_LIST) | awk -F ':.*## ' '{printf "  %-10s %s\n", $$1, $$2}'

run: ## Run the desktop app (debug build)
	cargo run

demo: release ## Run the app on a fresh demo database
	@db="$$(mktemp -d)/solilog.db" && scripts/demo-db.sh "$$db" && SOLILOG_DB="$$db" target/release/solilog

screenshot: release ## Update docs/screenshot.png (the README image) from the demo data (macOS)
	scripts/screenshot.sh

test: ## Run all tests
	cargo test --workspace

test-core: ## Run the core tests only (fast, no gpui)
	cargo test -p solilog-core

clippy: ## Lint, warnings as errors
	cargo clippy --workspace --all-targets -- -D warnings

check: test clippy ## Tests and lints

release: ## Release build of the binary
	cargo build --release -p solilog

bundle: release ## Build target/release/Solilog.app (macOS)
ifneq ($(UNAME),Darwin)
	@echo "bundle: macOS only (on Linux, \`make install\` installs the binary)" >&2; exit 1
endif
	rm -rf $(APP)
	mkdir -p $(APP)/Contents/MacOS $(APP)/Contents/Resources
	cp target/release/solilog $(APP)/Contents/MacOS/solilog
	cp $(MACOS)/Solilog.icns $(APP)/Contents/Resources/Solilog.icns
	sed 's/@VERSION@/$(VERSION)/g' $(MACOS)/Info.plist > $(APP)/Contents/Info.plist
	codesign --force --sign - $(APP)  # ad-hoc, so macOS treats it as one app

ifeq ($(UNAME),Darwin)
install: bundle ## Install the app and the `solilog` command
	-osascript -e 'if application "Solilog" is running then quit app "Solilog"'
	rm -rf "$(APP_DIR)/Solilog.app"
	ditto $(APP) "$(APP_DIR)/Solilog.app"
	mkdir -p "$(BIN_DIR)"
	ln -sf "$(APP_DIR)/Solilog.app/Contents/MacOS/solilog" "$(BIN_DIR)/solilog"
	-"$(BIN_DIR)/solilog" skill install
	@case ":$$PATH:" in *":$(BIN_DIR):"*) ;; *) echo "note: $(BIN_DIR) is not on your PATH";; esac
else
install: release
	mkdir -p "$(BIN_DIR)" "$(dir $(DESKTOP))" "$(dir $(ICON))"
# Replaced, not overwritten in place: a running app keeps its copy.
	rm -f "$(BIN_DIR)/solilog"
	cp target/release/solilog "$(BIN_DIR)/solilog"
	sed 's|@BIN@|$(BIN_DIR)/solilog|' $(LINUX)/$(APP_ID).desktop > "$(DESKTOP)"
	cp packaging/icon.svg "$(ICON)"
	@# Refresh the launcher's caches, where the desktop keeps them.
	@if command -v update-desktop-database >/dev/null; then update-desktop-database "$(dir $(DESKTOP))"; fi
	@if command -v gtk-update-icon-cache >/dev/null; then gtk-update-icon-cache -q -t "$(DATA_DIR)/icons/hicolor"; fi
	-"$(BIN_DIR)/solilog" skill install
	@case ":$$PATH:" in *":$(BIN_DIR):"*) ;; *) echo "note: $(BIN_DIR) is not on your PATH";; esac
endif

uninstall: ## Remove the app and the `solilog` command (the database is kept)
ifeq ($(UNAME),Darwin)
	rm -rf "$(APP_DIR)/Solilog.app"
else
	rm -f "$(DESKTOP)" "$(ICON)"
endif
	rm -f "$(BIN_DIR)/solilog"

icon: ## Regenerate the macOS .icns from packaging/icon.svg (needs rsvg-convert)
	$(MACOS)/icon.sh

clean: ## Remove build output
	cargo clean
