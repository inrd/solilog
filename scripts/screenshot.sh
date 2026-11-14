#!/bin/sh
# Opens the app on a fresh demo database (scripts/demo-db.sh) and saves a
# screenshot of its window to docs/screenshot.png, the README image.
# Run it (`make screenshot`) after changing how the app looks.
#
# macOS only. The terminal needs the Screen Recording permission (System
# Settings > Privacy & Security), or the capture shows only the desktop.
set -eu
cd "$(dirname "$0")/.."
s="${SOLILOG:-target/release/solilog}"
out=docs/screenshot.png

tmp="$(mktemp -d)"
stop() { [ -z "$pid" ] || { kill "$pid"; wait "$pid" || true; } 2>/dev/null; pid=""; }
trap 'stop; rm -rf "$tmp"' EXIT
pid=""

SOLILOG="$s" scripts/demo-db.sh "$tmp/solilog.db"

# Its own config dir, so a personal keymap can't change anything.
SOLILOG_DB="$tmp/solilog.db" SOLILOG_CONFIG_DIR="$tmp/config" "$s" &
pid=$!

tries=0
until id="$(swift scripts/window-id.swift "$pid" 2>/dev/null)"; do
    tries=$((tries + 1))
    [ "$tries" -lt 30 ] || { echo "the app window didn't appear" >&2; exit 1; }
    sleep 0.5
done
sleep 1 # let it render

mkdir -p docs
screencapture -x -l "$id" "$out"
stop
echo "wrote $out"
