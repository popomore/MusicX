#!/usr/bin/env bash
set -euo pipefail

tag=${RELEASE_TAG:-${GITHUB_REF_NAME:?}}
repo=${GITHUB_REPOSITORY:?}
source_sha=${SOURCE_SHA:?}
signing_mode=${SIGNING_MODE:?}
script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
cd "$(dirname "$1")"
dmg=$(basename "$1")
checksum="$dmg.sha256"
manifest='release-manifest.json'

title="MusicX ${tag#v}"
notes="Download MusicX for Apple Silicon and drag the app to Applications. Requires macOS 14 or later."
prerelease=true
case "$signing_mode" in
    ad-hoc)
        notes+=$'\n\nThis preview uses ad hoc signing. macOS may block it on first launch because it lacks Apple notarization.'
        ;;
    developer-id)
        if [[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
            prerelease=false
        fi
        ;;
    *)
        echo "::error::Unknown macOS signing mode."
        exit 1
        ;;
esac
if [[ "$prerelease" == true ]]; then
    title+=" (Preview)"
fi

sha256sum --check "$checksum"
files=("$dmg" "$checksum")
if [[ "$prerelease" == false ]]; then
    python3 "$script_dir/validate-appcast.py" "$dmg" "$tag" "$repo"
    files+=(appcast.xml)
fi
verify_tag() {
    local tag_sha
    tag_sha=$(gh api "repos/$repo/commits/refs/tags/$tag" --jq .sha)
    if [[ "$tag_sha" != "$source_sha" ]]; then
        echo "::error::The release tag no longer points to the packaged build."
        exit 1
    fi
}
verify_tag

file_digests() {
    sha256sum "$@" | jq -Rn '
        [inputs | split("  ") | {name: .[1], digest: ("sha256:" + .[0]), state: "uploaded"}]
        | sort_by(.name)'
}

jq -n --arg tag "$tag" --arg version "${tag#v}" --arg source_sha "$source_sha" \
    --arg signing_mode "$signing_mode" --argjson files "$(file_digests "${files[@]}")" '
    {schema: 1, unit: "musicx-macos", tag: $tag, version: $version, source_sha: $source_sha,
     signing_mode: $signing_mode, files: ($files | map({name, sha256: (.digest | ltrimstr("sha256:"))}))}
' > "$manifest"
files+=("$manifest")
expected=$(file_digests "${files[@]}")

release=$(gh api --paginate "repos/$repo/releases?per_page=100" |
    jq -cs --arg tag "$tag" '
        [.[][] | select(.tag_name == $tag)]
        | if length > 1 then error("Multiple releases match the tag") else .[0] end')
if [[ "$release" == null ]]; then
    gh release create "$tag" --repo "$repo" --verify-tag --draft \
        --prerelease="$prerelease" --title "$title" --notes "$notes"
    release=$(gh release view "$tag" --repo "$repo" --json databaseId,isDraft,isPrerelease |
        jq -c '{id: .databaseId, draft: .isDraft, prerelease: .isPrerelease}')
fi
if ! jq -e --argjson prerelease "$prerelease" '.prerelease == $prerelease' <<< "$release" > /dev/null; then
    echo "::error::The existing release type does not match this tag and signing mode."
    exit 1
fi
release_id=$(jq -er .id <<< "$release")

release_assets() {
    gh api --paginate "repos/$repo/releases/$release_id/assets?per_page=100" |
        jq -cs '[.[][] | {name, digest, state}] | sort_by(.name)'
}

if jq -e '.draft' <<< "$release" > /dev/null; then
    gh api --paginate "repos/$repo/releases/$release_id/assets?per_page=100" |
        jq -rs --argjson expected "$expected" '
            .[][] | select(.state == "starter")
            | select(.name as $name | any($expected[]; .name == $name)) | .id' |
        while IFS= read -r asset_id; do
            gh api --method DELETE "repos/$repo/releases/assets/$asset_id"
        done
fi

actual=$(release_assets)
if jq -e '.draft' <<< "$release" > /dev/null; then
    if ! jq -e --argjson expected "$expected" '. - $expected == []' <<< "$actual" > /dev/null; then
        echo "::error::The release draft contains unexpected assets or digests."
        exit 1
    fi
    missing=()
    for file in "${files[@]}"; do
        if ! jq -e --arg name "$file" 'any(.[]; .name == $name)' <<< "$actual" > /dev/null; then
            missing+=("$file")
        fi
    done
    if (( ${#missing[@]} )); then
        verify_tag
        gh release upload "$tag" --repo "$repo" "${missing[@]}"
        actual=$(release_assets)
    fi
fi

if ! jq -e --argjson expected "$expected" '. == $expected' <<< "$actual" > /dev/null; then
    echo "::error::GitHub release assets do not match the packaged build."
    exit 1
fi

verify_tag
if jq -e '.draft' <<< "$release" > /dev/null; then
    gh api --method PATCH "repos/$repo/releases/$release_id" -F draft=false --jq .html_url
else
    echo "Release $tag already contains the verified assets."
fi
