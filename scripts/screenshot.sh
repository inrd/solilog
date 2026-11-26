#!/bin/sh
# Opens the app on a fresh demo database (scripts/demo-db.sh) and saves
# screenshots of its window to docs/, for the README.
# Run it (`make screenshot`) after changing how the app looks.
#
# macOS only. The terminal needs the Screen Recording permission (System
# Settings > Privacy & Security), or the capture shows only the desktop.
set -eu
cd "$(dirname "$0")/.."
s="${SOLILOG:-target/release/solilog}"

tmp="$(mktemp -d)"
stop() { [ -z "$pid" ] || { kill "$pid"; wait "$pid" || true; } 2>/dev/null; pid=""; }
trap 'stop; rm -rf "$tmp"' EXIT
pid=""

SOLILOG="$s" scripts/demo-db.sh "$tmp/solilog.db"
mkdir -p docs

# capture <output path> <theme or empty> [issue id to open]
capture() {
    out="$1" theme="$2" open_issue="${3:-}"
    db="$tmp/shot.db"
    cp "$tmp/solilog.db" "$db"
    if [ -n "$theme" ]; then
        sqlite3 "$db" "INSERT OR REPLACE INTO settings (key, value) VALUES ('theme', '$theme')"
    fi

    # Its own config dir, so a personal keymap or user theme can't change anything.
    SOLILOG_DB="$db" SOLILOG_CONFIG_DIR="$tmp/config" SOLILOG_OPEN_ISSUE="$open_issue" "$s" &
    pid=$!

    tries=0
    until id="$(swift scripts/window-id.swift "$pid" 2>/dev/null)"; do
        tries=$((tries + 1))
        [ "$tries" -lt 30 ] || { echo "the app window didn't appear" >&2; exit 1; }
        sleep 0.5
    done
    sleep 1 # let it render

    screencapture -x -l "$id" "$out"
    stop
    rm -f "$db" "$db-wal" "$db-shm"
    echo "wrote $out"
}

capture docs/screenshot.png        ""     ""
capture docs/screenshot-detail.png ""     16
capture docs/screenshot-8086.png   "8086" ""
capture docs/screenshot-mmx.png    "mmx"  ""
