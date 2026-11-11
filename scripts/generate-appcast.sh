#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
: "${SPARKLE_PRIVATE_KEY:?Configure the Sparkle Ed25519 signing key}"
: "${GITHUB_REPOSITORY:?}"
: "${RELEASE_TAG:=${GITHUB_REF_NAME:?}}"
if [[ ! "$RELEASE_TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo 'Only stable releases may publish the automatic update feed.' >&2
    exit 1
fi
dmg=$1
sparkle=$(bash scripts/prepare-sparkle.sh)
stage=$(mktemp -d "${TMPDIR:-/tmp}/musicx-appcast.XXXXXX")
trap 'rm -rf "$stage"' EXIT
# Isolate the one verified release; do not include preview or stale archives.
cp "$dmg" "$stage/"
printf '%s' "$SPARKLE_PRIVATE_KEY" | "$sparkle/bin/generate_appcast" \
    --ed-key-file - --maximum-deltas 0 \
    --download-url-prefix "https://github.com/$GITHUB_REPOSITORY/releases/download/$RELEASE_TAG/" \
    --link "https://github.com/$GITHUB_REPOSITORY/releases/tag/$RELEASE_TAG" "$stage"
test -s "$stage/appcast.xml"
cp "$stage/appcast.xml" "$(dirname "$dmg")/appcast.xml"
