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
export SOLILOG_NO_SKILL_HINT=1
s() { "$bin" "$@" >/dev/null; }
# `--actor`, not `SOLILOG_ACTOR=… s`: sh keeps a variable set before a
# function call, so every later write would be the agent's.

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
s --actor claude:fix-header issue edit SOL-2 -d "Investigated flexbox wrap."
s issue review SOL-1
# Notes (0.2.3+): one by the user, one by the agent on SOL-2.
s note add SOL-2 "Check it on a phone too."
s --actor claude:fix-header note add SOL-2 "Tried flex-wrap.
The logo still overflows."
# Review handoff (0.2.3+): SOL-4 handed off, sent back, handed off again.
s issue create "Update the footer" -l docs
s --actor claude:footer issue request-review SOL-4 -m "Footer updated." \
    --pass "make test" --fail "make lint" --commit abc1234 --file footer.html
s issue send-back SOL-4 -m "Fix the lint first."
s --actor claude:footer issue request-review SOL-4 -m "Lint fixed." --pass "make lint"
# Mapped paths (after 0.2.4): one under the home directory, kept as `~/…`, and
# one absolute; neither exists.
s label add-path web "~/code/website"
s project add-path "Website v2" /srv/solilog-fixture/website-v2
# Decisions (after 0.2.4): one on a label, replaced; one by an agent on
# SOL-2, unreviewed.
s decision add --label web "Keep the header under 80px."
s --actor claude:fix-header decision add --issue SOL-2 "The logo scales down below 600px."
s decision replace 1 "Keep the header under 64px."

mkdir -p "$(dirname "$out")"
# A single file: every connection is closed, so the WAL has been merged.
[ ! -e "$SOLILOG_DB-wal" ] || { echo "upgrade-fixture: $SOLILOG_DB-wal left over" >&2; exit 1; }
cp "$SOLILOG_DB" "$out"
echo "Wrote $out"
