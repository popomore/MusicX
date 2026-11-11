#!/usr/bin/env bash
# Keep a signed feed at one fixed GitHub release, independent of Latest.
set -euo pipefail
cd "$(dirname "$0")/.."
dmg=${1:?}
repo=${GITHUB_REPOSITORY:?}
tag=${RELEASE_TAG:-${GITHUB_REF_NAME:?}}
sparkle=$(bash scripts/prepare-sparkle.sh)
stage=$(mktemp -d "${TMPDIR:-/tmp}/musicx-update-feed.XXXXXX")
trap 'rm -rf "$stage"' EXIT
directory=$(dirname "$dmg")
previous=$(gh api --paginate "repos/$repo/releases?per_page=100" | jq -rs '
    [.[][] | select(.draft == false)] as $releases |
    ($releases | map(select(.tag_name == "update-feed")) | first) //
    ($releases | map(select(.prerelease == false and (.tag_name | test("^v[0-9]+\\.[0-9]+\\.[0-9]+$"))))
     | sort_by(.published_at) | last)')
prior='-'
base=missing
if [[ "$previous" != null ]]; then
    previous_tag=$(jq -er .tag_name <<< "$previous")
    mkdir "$stage/prior"
    gh release download "$previous_tag" --repo "$repo" --pattern appcast.xml --dir "$stage/prior"
    prior="$stage/prior/appcast.xml"
    if [[ "$previous_tag" == update-feed ]]; then
        base=$(shasum -a 256 "$prior" | cut -d ' ' -f 1)
    fi
fi
python3 scripts/merge-appcast.py "$directory/appcast.xml" "$prior" "$directory/appcast-all.xml"
printf '%s' "${SPARKLE_PRIVATE_KEY:?}" | "$sparkle/bin/sign_update" --ed-key-file - "$directory/appcast-all.xml"
python3 scripts/validate-appcast.py "$dmg" "$tag" "$repo" --all
printf '%s\n' "$base" > "$directory/appcast-all.base"
