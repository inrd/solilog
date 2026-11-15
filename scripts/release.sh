#!/bin/sh
# Cut a release: scripts/release.sh 0.2.0 (or `make cut-release V=0.2.0`).
#
# Needs a clean working tree and release notes under `## [Unreleased]` in
# CHANGELOG.md. Moves those notes to a `## [0.2.0] - <date>` section, sets the
# workspace version in Cargo.toml (and Cargo.lock), then commits
# "Release v0.2.0" and tags it v0.2.0. The tests and lints run first. Nothing
# is pushed.
set -eu

cd "$(dirname "$0")/.."

die() { echo "release: $*" >&2; exit 1; }

[ $# -eq 1 ] || die "usage: scripts/release.sh <version>  (e.g. 0.2.0)"
new=$1
tag="v$new"

echo "$new" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' || die "'$new' is not a version like 0.2.0"
[ -z "$(git status --porcelain)" ] || die "the working tree has uncommitted changes"
git rev-parse -q --verify "refs/tags/$tag" >/dev/null && die "tag $tag already exists"

# The first `version = ` line of Cargo.toml is [workspace.package]'s.
current=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
[ -n "$current" ] || die "no workspace version in Cargo.toml"
# Equal is allowed for a version that was never tagged (the first release).
lowest=$(printf '%s\n%s\n' "$current" "$new" | sort -t. -k1,1n -k2,2n -k3,3n | head -n 1)
[ "$lowest" = "$current" ] || die "$new is lower than the current version $current"

# The release notes: lines between `## [Unreleased]` and the next section.
grep -q '^## \[Unreleased\]' CHANGELOG.md || die "CHANGELOG.md has no '## [Unreleased]' section"
notes=$(awk '/^## \[Unreleased\]/ { on = 1; next } /^## / { on = 0 } on && NF' CHANGELOG.md)
[ -n "$notes" ] || die "the [Unreleased] section of CHANGELOG.md is empty: write the release notes first"

# Before changing anything, so a failure leaves the tree as it was.
make check

date=$(date +%Y-%m-%d)
tmp=$(mktemp)
trap 'rm -f "$tmp"' EXIT

awk -v section="## [$new] - $date" '
    /^## \[Unreleased\]/ && !done { print; print ""; print section; done = 1; next }
    { print }
' CHANGELOG.md > "$tmp" && cp "$tmp" CHANGELOG.md

awk -v v="$new" '
    /^version = "/ && !done { print "version = \"" v "\""; done = 1; next }
    { print }
' Cargo.toml > "$tmp" && cp "$tmp" Cargo.toml

# Records the new version of the workspace crates in Cargo.lock.
cargo update --workspace --quiet

git add CHANGELOG.md Cargo.toml Cargo.lock
git commit --quiet -m "Release $tag"
git tag -a "$tag" -m "Solilog $new"

echo "Tagged $tag. To publish it: git push origin HEAD $tag"
