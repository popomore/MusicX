#!/usr/bin/env bash
set -euo pipefail
script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
dmg=${1:?}
repo=${GITHUB_REPOSITORY:?}
tag=${RELEASE_TAG:-${GITHUB_REF_NAME:?}}
python3 "$script_dir/validate-appcast.py" "$dmg" "$tag" "$repo" --all
directory=$(dirname "$dmg")
base=$(cat "$directory/appcast-all.base")
digest=$(sha256sum "$directory/appcast-all.xml" | cut -d ' ' -f 1)
release=$(gh api --paginate "repos/$repo/releases?per_page=100" |
    jq -rs '[.[][] | select(.tag_name == "update-feed")] |
        if length > 1 then error("Multiple update feeds") else .[0] end')
stage=$(mktemp -d "${TMPDIR:-/tmp}/musicx-publish-feed.XXXXXX")
trap 'rm -rf "$stage"' EXIT
cp "$directory/appcast-all.xml" "$stage/appcast.xml"
if [[ "$release" == null ]]; then
    [[ "$base" == missing ]] || { echo 'Update feed was removed; refusing to recreate it.' >&2; exit 1; }
    gh release create update-feed --repo "$repo" --target "${SOURCE_SHA:?}" \
        --prerelease --latest=false --title 'MusicX Update Feed' \
        --notes 'Signed update metadata for MusicX. Download installers from the versioned releases.' \
        "$stage/appcast.xml"
else
    jq -e '.draft == false and .prerelease == true and .immutable != true' <<< "$release" >/dev/null
    actual=$(gh api --paginate "repos/$repo/releases/$(jq -er .id <<< "$release")/assets?per_page=100" |
        jq -rs '[.[][] | select(.name == "appcast.xml")] |
            if length == 1 and .[0].state == "uploaded" then .[0].digest else error("Invalid update feed assets") end')
    if [[ "$actual" == "sha256:$digest" ]]; then
        echo 'The signed update feed is already published.'
        exit 0
    fi
    [[ "$actual" == "sha256:$base" ]] || { echo 'Update feed changed since packaging; refusing a stale replacement.' >&2; exit 1; }
    gh release upload update-feed --repo "$repo" --clobber "$stage/appcast.xml"
fi
published=$(gh api "repos/$repo/releases/tags/update-feed" --jq .id)
gh api --paginate "repos/$repo/releases/$published/assets?per_page=100" |
    jq -es --arg digest "sha256:$digest" 'any(.[][]; .name == "appcast.xml" and .state == "uploaded" and .digest == $digest)' >/dev/null
