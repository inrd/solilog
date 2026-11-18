#!/bin/sh
# Fills a new database with neutral demo data (a fictional notes app), through
# the CLI. Used for the README screenshot (`make screenshot`) and to try the
# app (`make demo`). Keep it fictional: no real people, companies or projects.
#
#   scripts/demo-db.sh <database path>     (must not exist yet)
#   SOLILOG=path/to/solilog                (default: target/release/solilog)
set -eu
[ $# -eq 1 ] || { echo "usage: $0 <database path>" >&2; exit 2; }
[ ! -e "$1" ] || { echo "$1 already exists" >&2; exit 1; }
export SOLILOG_DB="$1"
s="${SOLILOG:-target/release/solilog}"

q() { "$s" "$@" > /dev/null; }

# issue <status> <checked items> <title> [create options…]
# Within a status, issues created last are listed first.
issue() {
    status="$1" checked="$2" title="$3"
    shift 3
    json="$("$s" --json issue create "$title" -s "$status" "$@")"
    i=0
    while [ "$i" -lt "$checked" ]; do
        q checklist check "$(printf '%s' "$json" | plutil -extract "checklist.$i.id" raw -)"
        i=$((i + 1))
    done
}

# Labels, in sidebar order.
for l in bug feature design docs platform/ios platform/web platform/desktop; do q label create "$l"; done
q label color bug red
q label color feature blue
q label color design purple
q label color docs gray
q label color platform/ios teal
q label color platform/web green
q label color platform/desktop orange

q project create "Public beta" -d "Open the app to everyone on the waiting list."
q project create "Offline mode" -d "Every note readable and editable without a connection." -l platform/desktop
q project create "Design refresh" -s planned -l design
q project create "Onboarding" -s done

q view save "Open bugs" --open -l bug
q view save "In flight" -s in_progress -s blocked
q view save "Web" --open -l platform/web

issue cancelled 0 "Browser extension" -l feature -l platform/web
issue done 3 "Pricing page" -l platform/web -p "Public beta" -i Copy -i Layout -i "Stripe checkout"
issue done 0 "Welcome tour" -p Onboarding -l design
issue done 0 "Set up crash reporting" -l platform/ios -l platform/desktop

issue backlog 0 "Localize into French and German" -l feature
issue backlog 0 "Tablet layout" -l platform/ios -l design
issue backlog 0 "Self-hosted sync server" -l feature

issue blocked 1 "Privacy labels for the App Store review" -l platform/ios -p "Public beta" \
    -i "List collected data" -i "Answer the review team"

issue todo 0 "Write the beta announcement" -l docs -p "Public beta"
issue todo 0 "Crash when opening a file from the share sheet" -l bug -l platform/ios
issue todo 0 "Dark mode for the marketing site" -l design -l platform/web
issue todo 0 "Keyboard shortcuts cheat sheet" -l docs
issue todo 0 "Export notes to Markdown" -l feature -p "Offline mode"

issue in_progress 1 "Search results page" -l feature -l platform/web -p "Public beta" \
    -i "Ranking" -i "Highlight matches" -i "Empty state"
issue in_progress 0 "Redesign the settings screen" -p "Design refresh" -l platform/desktop
issue in_progress 2 "Sync conflicts lose the newest edit" -l bug -p "Offline mode" \
    -i "Reproduce with two devices" -i "Write a failing test" -i "Merge by paragraph" -i "Ship a fix" \
    -d "Editing the same note on two devices while offline keeps the older version when they reconnect.

Needed before SOL-7." # "Self-hosted sync server", the 7th issue created

# Issue numbers follow the creation order above.
q issue add-dependency 7 16  # the sync server needs "Sync conflicts…"
q issue add-dependency 9 14 8  # the announcement waits for search and the review

q statuses collapse backlog
