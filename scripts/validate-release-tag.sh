#!/usr/bin/env bash
# Validate a release tag and emit its version and channel.
set -euo pipefail

source_sha=${1:?}
tag=${2:-}
source_sha=$(git rev-parse --verify "$source_sha^{commit}")
number='(0|[1-9][0-9]*)'
version_pattern="$number\\.$number\\.$number"

if [[ "$tag" =~ ^v($version_pattern)$ ]]; then
    version=${BASH_REMATCH[1]}
    channel=stable
elif [[ "$tag" =~ ^v($version_pattern)\.preview-([0-9]{14})-([0-9a-f]{4})$ ]]; then
    version=${BASH_REMATCH[1]}
    tag_timestamp=${BASH_REMATCH[5]}
    tag_sha=${BASH_REMATCH[6]}
    timestamp=$(TZ=UTC git show -s --format=%cd --date=format-local:%Y%m%d%H%M%S "$source_sha")
    if [[ "$tag_timestamp" != "$timestamp" || "$tag_sha" != "${source_sha:0:4}" ]]; then
        echo '::error::The preview tag timestamp and SHA must match the source commit.' >&2
        exit 1
    fi
    channel=preview
else
    echo '::error::Invalid release tag. Expected vX.Y.Z or vX.Y.Z.preview-YYYYMMDDHHMMSS-SHA4, without leading zeroes.' >&2
    exit 1
fi

case "${3:-}" in
    '') printf '%s\n' "$channel" ;;
    --github-output) printf 'sha=%s\nchannel=%s\nversion=%s\nbundle-version=%s\n' "$source_sha" "$channel" "${tag#v}" "$version" ;;
    *)
        echo '::error::Unknown validation output option.' >&2
        exit 1
        ;;
esac
