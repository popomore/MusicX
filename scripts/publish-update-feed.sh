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
restore_live=false
live_id=''
release_assets() {
    gh api --paginate "repos/$repo/releases/$release_id/assets?per_page=100" | jq -cs '[.[][]]'
}
live_asset() {
    jq -c '[.[] | select(.name == "appcast.xml" and .state == "uploaded")] |
        if length > 1 then error("Multiple live feeds") else .[0] end'
}
rename_asset() {
    gh api --method PATCH "repos/$repo/releases/assets/$1" -f name="$2" --silent
}
finish() {
    local result=$? assets live
    trap - EXIT
    if [[ "$result" != 0 && "$restore_live" == true ]]; then
        # Renaming preserves the signed bytes remotely, even if this process dies.
        # Never replace a newer live feed after an ambiguous API response.
        if assets=$(release_assets) && live=$(live_asset <<< "$assets"); then
            if [[ "$live" == null ]] && jq -e --argjson id "$live_id" --arg digest "sha256:$base" \
                'any(.[]; .id == $id and .state == "uploaded" and .digest == $digest)' <<< "$assets" >/dev/null; then
                if rename_asset "$live_id" appcast.xml && \
                    release_assets | live_asset | jq -e --arg digest "sha256:$base" '.digest == $digest' >/dev/null; then
                    echo 'Restored the previous signed update feed.' >&2
                else
                    echo 'Previous signed feed remains in appcast-previous-'"$live_id"'.xml; retry publication to recover.' >&2
                fi
            fi
        else
            echo 'Could not confirm live feed state; retained the remote signed backup for recovery.' >&2
        fi
    fi
    rm -rf "$stage"
    exit "$result"
}
trap finish EXIT
cp "$directory/appcast-all.xml" "$stage/appcast.xml"
if [[ "$release" == null ]]; then
    [[ "$base" == missing ]] || { echo 'Update feed was removed; refusing to recreate it.' >&2; exit 1; }
    gh release create update-feed --repo "$repo" --target "${SOURCE_SHA:?}" \
        --prerelease --latest=false --title 'MusicX Update Feed' \
        --notes 'Signed update metadata for MusicX. Download installers from the versioned releases.' \
        "$stage/appcast.xml"
else
    jq -e '.draft == false and .prerelease == true and .immutable != true' <<< "$release" >/dev/null
    release_id=$(jq -er .id <<< "$release")
    assets=$(release_assets)
    live=$(live_asset <<< "$assets")
    if [[ "$live" == null ]]; then
        # A runner may have stopped between the two renames. Recover the exact
        # signed base chosen during packaging before attempting another swap.
        recovery=$(jq -c --arg digest "sha256:$base" '
            [.[] | select(.state == "uploaded" and .digest == $digest and
                (.name | startswith("appcast-previous-") or startswith("appcast-next-")))] |
            sort_by(.id) | last' <<< "$assets")
        [[ "$recovery" != null ]] || { echo 'No matching signed feed is available for recovery.' >&2; exit 1; }
        incomplete=$(jq -r '.[] | select(.name == "appcast.xml" and .state == "starter") | .id' <<< "$assets")
        if [[ -n "$incomplete" ]]; then rename_asset "$incomplete" "appcast-incomplete-$incomplete.xml"; fi
        rename_asset "$(jq -er .id <<< "$recovery")" appcast.xml
        assets=$(release_assets)
        live=$(live_asset <<< "$assets")
    fi
    actual=$(jq -er .digest <<< "$live")
    if [[ "$actual" == "sha256:$digest" ]]; then
        echo 'The signed update feed is already published.'
        exit 0
    fi
    [[ "$actual" == "sha256:$base" ]] || { echo 'Update feed changed since packaging; refusing a stale replacement.' >&2; exit 1; }
    live_id=$(jq -er .id <<< "$live")
    candidate=$(jq -c --arg digest "sha256:$digest" '
        [.[] | select(.state == "uploaded" and .digest == $digest and
            (.name | startswith("appcast-next-")))] | sort_by(.id) | last' <<< "$assets")
    if [[ "$candidate" == null ]]; then
        name="appcast-next-$digest-${stage##*.}.xml"
        cp "$stage/appcast.xml" "$stage/$name"
        # Upload and authenticate the replacement before touching the live asset.
        gh release upload update-feed --repo "$repo" "$stage/$name"
        candidate=$(release_assets | jq -ec --arg name "$name" --arg digest "sha256:$digest" '
            .[] | select(.name == $name and .state == "uploaded" and .digest == $digest)')
    fi
    # Recheck the base after staging, before either rename.
    release_assets | live_asset | jq -e --argjson id "$live_id" --arg digest "sha256:$base" \
        '.id == $id and .digest == $digest' >/dev/null
    restore_live=true
    rename_asset "$live_id" "appcast-previous-$live_id.xml"
    rename_asset "$(jq -er .id <<< "$candidate")" appcast.xml
fi
published=$(gh api "repos/$repo/releases/tags/update-feed" --jq .id)
gh api --paginate "repos/$repo/releases/$published/assets?per_page=100" |
    jq -es --arg digest "sha256:$digest" 'any(.[][]; .name == "appcast.xml" and .state == "uploaded" and .digest == $digest)' >/dev/null
restore_live=false
