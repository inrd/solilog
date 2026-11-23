#!/bin/sh
# Make the upgrade fixture of a release: scripts/upgrade-fixture.sh 0.2.0
# (run by scripts/release.sh).
#
# Builds the current tree and fills a fresh database with the same data
# through the CLI, saved as crates/solilog-core/tests/fixtures/v0.2.0.db.
# The `upgrades` test opens every fixture with the current code and checks
# that this data survived the migrations. Keep the data in step with that
# test: add to it for new features, never change what is there.
set -eu

cd "$(dirname "$0")/.."

[ $# -eq 1 ] || { echo "usage: scripts/upgrade-fixture.sh <version>" >&2; exit 1; }
out="crates/solilog-core/tests/fixtures/v$1.db"

cargo build --quiet --bin solilog
bin="$PWD/target/debug/solilog"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
export SOLILOG_DB="$work/fixture.db"
export SOLILOG_ACTOR=user
s() { "$bin" "$@" >/dev/null; }

s project create "Website v2" -d "Launch the new site" -l web
s project create "Old site" -s done
s issue create "Write the post" -s in_progress -l docs -p "Website v2" \
    -d "Two paragraphs.

Mention SOL-2." -i "Draft" -i "Review" -i "Proofread"
s checklist check 1
# Revisions (0.2.0+): the old title, and a deleted item.
s issue edit SOL-1 --title "Write the launch post"
s checklist delete 3
s issue create "Fix the header layout" -s blocked -l web/frontend
printf 'header notes\n' > "$work/notes.txt"
s attachment add SOL-2 "$work/notes.txt"
s issue create "Old idea" -s cancelled
s issue archive SOL-3
s label color web teal
s label collapse web
s view save "Open web" --open -l web
# Dependencies (0.2.1+): the post waits for the header fix.
s issue add-dependency SOL-1 SOL-2
# Authorship & review (0.2.2+): an agent edit on SOL-2, unreviewed; SOL-1 reviewed.
SOLILOG_ACTOR=claude:fix-header s issue edit SOL-2 -d "Investigated flexbox wrap."
s issue review SOL-1
# Notes (0.2.3+): one by the user, one by the agent on SOL-2.
s note add SOL-2 "Check it on a phone too."
SOLILOG_ACTOR=claude:fix-header s note add SOL-2 "Tried flex-wrap.
The logo still overflows."

mkdir -p "$(dirname "$out")"
# A single file: every connection is closed, so the WAL has been merged.
[ ! -e "$SOLILOG_DB-wal" ] || { echo "upgrade-fixture: $SOLILOG_DB-wal left over" >&2; exit 1; }
cp "$SOLILOG_DB" "$out"
echo "Wrote $out"
