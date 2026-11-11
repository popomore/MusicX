#!/usr/bin/env bash
set -euo pipefail

source_sha=${1:?}
tag=${2:-}
metadata=$(cargo metadata --locked --no-deps --format-version 1)
version=$(jq -er '.packages[] | select(.name == "musicx") | .version' <<< "$metadata")
bundle_version=$(jq -er '.packages[] | select(.name == "musicx-platform-apple") | .version' <<< "$metadata")
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ || "$version" != "$bundle_version" ]]; then
    echo '::error::Both MusicX Cargo manifests must specify the same X.Y.Z version.' >&2
    exit 1
fi

timestamp=$(TZ=UTC git show -s --format=%cd --date=format-local:%Y%m%d%H%M%S "$source_sha")
preview_tag="v${version}.preview-$timestamp-${source_sha:0:4}"
case "$tag" in
    '') printf '%s\n' "$preview_tag" ;;
    "$preview_tag") printf 'preview\n' ;;
    "v$version") printf 'stable\n' ;;
    *)
        printf '::error::Expected release tag v%s or %s.\n' "$version" "$preview_tag" >&2
        exit 1
        ;;
esac
