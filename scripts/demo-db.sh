#!/bin/sh
# Fills a new database with neutral, cross-project demo data through the CLI.
# Used for the README screenshots (`make screenshot`) and to try the app
# (`make demo`). Keep it fictional: no real people, companies or projects.
#
#   scripts/demo-db.sh <database path>     (must not exist yet)
#   SOLILOG=path/to/solilog                (default: target/release/solilog)
set -eu
[ $# -eq 1 ] || { echo "usage: $0 <database path>" >&2; exit 2; }
[ ! -e "$1" ] || { echo "$1 already exists" >&2; exit 1; }
export SOLILOG_DB="$1"
export SOLILOG_ACTOR=user
export SOLILOG_NO_SKILL_HINT=1
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
for l in bug feature design docs repo/api repo/web repo/cli; do q label create "$l"; done
q label color bug red
q label color feature blue
q label color design purple
q label color docs gray
q label color repo/api orange
q label color repo/web green
q label color repo/cli teal
# Where each codebase lives (needn't exist: missing folders are kept).
for r in api web cli; do q label add-path "repo/$r" "~/code/$r"; done

q project create "v2.0 launch" -d "Ship the new API and updated web client together."
q project create "CLI rewrite" -d "Replace the legacy Python scripts with a single binary." -l repo/cli
q project create "Search v2" -s planned -l repo/api
q project create "Billing migration" -s done

q view save "Open bugs" --open -l bug
q view save "In flight" -s in_progress -s blocked
q view save "CLI" --open -l repo/cli

# 1..4: closed
issue cancelled 0 "Browser extension" -l feature -l repo/web
issue done 3 "Webhook signing keys" -l repo/api -p "v2.0 launch" -i "Key rotation" -i "Signature header" -i "Docs"
issue done 0 "Invoice PDF export" -p "Billing migration" -l repo/api
issue done 0 "Shell completions" -l repo/cli -p "CLI rewrite"

# 5..7: backlog
issue backlog 0 "Localize web dashboard" -l feature -l repo/web
issue backlog 0 "Fuzzy match on archived records" -l feature -p "Search v2"
issue backlog 0 "Self-hosted worker image" -l feature -l repo/api

# 8: blocked
issue blocked 1 "OAuth app verification" -l repo/api -p "v2.0 launch" \
    -i "Submit scopes justification" -i "Confirm callback URLs"

# 9..12: todo
issue todo 0 "Write v2.0 migration guide" -l docs -p "v2.0 launch"
issue todo 0 "Stale cache after renaming a workspace" -l bug -l repo/web
issue todo 0 "Keyboard navigation in filter bar" -l design -l repo/web
issue todo 0 "Import config from legacy rc file" -l feature -p "CLI rewrite"

# 13..14: in progress
issue in_progress 1 "Streaming table output for watch mode" -l feature -p "CLI rewrite" \
    -i "Buffered renderer" -i "Resize handling" -i "Color fallback"
issue in_progress 1 "Retry backoff drops idempotency key" -l bug -l repo/api -p "v2.0 launch" \
    -i "Reproduce with 503 stub" -i "Preserve header on retry" -i "Add regression test" \
    -d "Retrying a failed POST after a 503 regenerates the idempotency key, causing duplicate writes.

Needed before SOL-7."

# Issue numbers follow the creation order above.
q issue add-dependency 7 14  # worker image waits on retry backoff fix
q issue add-dependency 9 14 8  # migration guide waits on retry fix and OAuth verification

# One in-progress issue updated by an agent, so the Unreviewed view and badge show up.
q --actor codex:retry-fix checklist check 10

# 15..16: two issues handed off for review by different agents across repos.
issue in_progress 2 "Structured JSON output for export" -l feature -p "CLI rewrite" \
    -i "NDJSON flag" -i "Schema tests"
q --actor cursor:cli-export issue request-review 15 \
    -m "Added --ndjson to export and covered empty and nested payloads." \
    --pass "cargo test -p cli" --commit 9c4d10a --file src/cmd/export.rs

# 16: showcase issue for the detail screenshot (description, checklist, dependency,
# cross-agent note, and review receipt).
issue in_progress 3 "Rate limiting on /v2/events" -l feature -l repo/api -p "v2.0 launch" \
    -i "Token bucket store" -i "Retry-After header on 429" -i "Integration tests" \
    -d "Enforce per-token limits on POST /v2/events with a Retry-After header on 429, keeping existing webhook delivery unaffected (see SOL-2)."
q issue add-dependency 16 2
q --actor codex:load-test note add 16 \
    "Benchmarked the in-memory limiter at 12k req/s; sliding window had 3x higher lock contention than a token bucket."
# Decisions: one on the API codebase, applying to its issues, and one an
# agent recorded on the issue, awaiting review.
q decision add --label repo/api \
    "Public endpoints stay backward compatible within v2; breaking changes wait for v3."
q --actor claude:rate-limit decision add --issue 16 \
    "Limits are per token, not per IP, so clients behind a shared NAT don't throttle each other."
q --actor claude:rate-limit issue request-review 16 \
    -m "Added per-token bucket limiter on /v2/events with Retry-After headers and burst tests." \
    --pass "cargo test -p api" --pass "cargo clippy --all-targets" \
    --commit 4e1b9d2 --file src/http/events.rs --file src/limits/bucket.rs

q statuses collapse backlog
