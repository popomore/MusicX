#!/usr/bin/env bash
set -euo pipefail

repo=${GITHUB_REPOSITORY:?}
source_sha=${SOURCE_SHA:?}
output=${GITHUB_OUTPUT:?}

if [[ "$(git rev-parse HEAD)" != "$source_sha" ]]; then
    echo "::error::The checkout does not match the preview build."
    exit 1
fi
tag=$(bash "$(dirname "$0")/release-tag.sh" "$source_sha")

existing=$(gh api "repos/$repo/git/matching-refs/tags/$tag" \
    --jq ".[] | select(.ref == \"refs/tags/$tag\") | .ref")
if [[ -n "$existing" ]]; then
    tag_sha=$(gh api "repos/$repo/commits/refs/tags/$tag" --jq .sha)
    if [[ "$tag_sha" != "$source_sha" ]]; then
        echo "::error::The preview tag already points to another commit."
        exit 1
    fi
else
    gh api --method POST "repos/$repo/git/refs" \
        -f ref="refs/tags/$tag" -f sha="$source_sha" --silent
fi

printf 'tag=%s\n' "$tag" >> "$output"
echo "Preview tag: $tag"
