#!/usr/bin/env bash
set -euo pipefail

repo=${GITHUB_REPOSITORY:?}
source_sha=${SOURCE_SHA:?}
output=${GITHUB_OUTPUT:?}

if [[ "$(git rev-parse HEAD)" != "$source_sha" ]]; then
    echo "::error::The checkout does not match the preview build."
    exit 1
fi
release_script="$(dirname "$0")/release-tag.sh"
published_tags=$(gh api --paginate "repos/$repo/releases?per_page=100" |
    jq -rs '[.[][] | select(.draft == false and .prerelease == false)]
        | sort_by(.published_at) | reverse | .[].tag_name')
stable_tag=''
while IFS= read -r candidate; do
    if channel=$(bash "$release_script" "$source_sha" "$candidate" 2>/dev/null) && [[ "$channel" == stable ]]; then
        stable_tag=$candidate
        break
    fi
done <<< "$published_tags"
if [[ -z "$stable_tag" ]]; then
    echo 'No published stable version; skipping the automatic preview.'
    exit 0
fi
timestamp=$(TZ=UTC git show -s --format=%cd --date=format-local:%Y%m%d%H%M%S "$source_sha")
tag="$stable_tag.preview-$timestamp-${source_sha:0:4}"
bash "$release_script" "$source_sha" "$tag" > /dev/null

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
