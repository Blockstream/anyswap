#!/bin/sh
# Prints a version's release notes: its section of each changelog given, under
# that changelog's heading. A changelog without a section for the version is
# left out. POSIX sh, since the GitLab release image has no bash.
#
# Usage: scripts/release-notes.sh <version> '<heading>|<changelog>'...
set -eu

usage="usage: $0 <version> '<heading>|<changelog>'..."
[ $# -ge 2 ] || {
    echo "$usage" >&2
    exit 1
}
version="$1"
shift

for entry in "$@"; do
    heading="${entry%%|*}"
    changelog="${entry#*|}"
    grep -qxF "## [$version]" "$changelog" || continue
    printf '## %s\n' "$heading"
    awk -v heading="## [$version]" '
        $0 == heading { on = 1; next }
        /^## \[/ { on = 0 }
        on' "$changelog"
done
