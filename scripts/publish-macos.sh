#!/usr/bin/env bash
set -euo pipefail

tag=${GITHUB_REF_NAME:?}
repo=${GITHUB_REPOSITORY:?}
source_sha=${SOURCE_SHA:?}
cd "$(dirname "$1")"
dmg=$(basename "$1")
checksum="$dmg.sha256"
manifest='release-manifest.json'

sha256sum --check "$checksum"
tag_sha=$(gh api "repos/$repo/commits/refs/tags/$tag" --jq .sha)
if [[ "$tag_sha" != "$source_sha" ]]; then
    echo "::error::The release tag no longer points to the signed build."
    exit 1
fi

file_digests() {
    sha256sum "$@" | jq -Rn '
        [inputs | split("  ") | {name: .[1], digest: ("sha256:" + .[0]), state: "uploaded"}]
        | sort_by(.name)'
}

jq -n --arg tag "$tag" --arg version "${tag#v}" --arg source_sha "$source_sha" \
    --argjson files "$(file_digests "$dmg" "$checksum")" '
    {schema: 1, unit: "musicx-macos", tag: $tag, version: $version, source_sha: $source_sha,
     files: ($files | map({name, sha256: (.digest | ltrimstr("sha256:"))}))}
' > "$manifest"
expected=$(file_digests "$dmg" "$checksum" "$manifest")

release=$(gh api --paginate "repos/$repo/releases?per_page=100" |
    jq -cs --arg tag "$tag" '
        [.[][] | select(.tag_name == $tag)]
        | if length > 1 then error("Multiple releases match the tag") else .[0] end')
if [[ "$release" == null ]]; then
    gh release create "$tag" --repo "$repo" --verify-tag --draft \
        --title "MusicX ${tag#v}" \
        --notes "Download MusicX for Apple Silicon and drag the app to Applications. Requires macOS 14 or later."
    release=$(gh release view "$tag" --repo "$repo" --json databaseId,isDraft |
        jq -c '{id: .databaseId, draft: .isDraft}')
fi
release_id=$(jq -er .id <<< "$release")

release_assets() {
    gh api --paginate "repos/$repo/releases/$release_id/assets?per_page=100" |
        jq -cs '[.[][] | {name, digest, state}] | sort_by(.name)'
}

actual=$(release_assets)
if jq -e '.draft' <<< "$release" > /dev/null; then
    if ! jq -e --argjson expected "$expected" '. - $expected == []' <<< "$actual" > /dev/null; then
        echo "::error::The release draft contains unexpected assets or digests."
        exit 1
    fi
    missing=()
    for file in "$dmg" "$checksum" "$manifest"; do
        if ! jq -e --arg name "$file" 'any(.[]; .name == $name)' <<< "$actual" > /dev/null; then
            missing+=("$file")
        fi
    done
    if (( ${#missing[@]} )); then
        gh release upload "$tag" --repo "$repo" "${missing[@]}"
        actual=$(release_assets)
    fi
fi

if ! jq -e --argjson expected "$expected" '. == $expected' <<< "$actual" > /dev/null; then
    echo "::error::GitHub release assets do not match the signed build."
    exit 1
fi

if jq -e '.draft' <<< "$release" > /dev/null; then
    gh api --method PATCH "repos/$repo/releases/$release_id" -F draft=false --jq .html_url
else
    echo "Release $tag already contains the verified assets."
fi
