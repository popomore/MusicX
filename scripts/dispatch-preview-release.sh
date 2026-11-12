#!/usr/bin/env bash
set -euo pipefail

repo=${GITHUB_REPOSITORY:?}
tag=${1:?}
published=$(gh api --paginate "repos/$repo/releases?per_page=100" |
    jq -cs --arg tag "$tag" '
        any(.[][];
            .tag_name == $tag and .draft == false and .prerelease == true and
            ([.assets[]? | select(.state == "uploaded" and
                (.digest // "" | test("^sha256:[0-9a-f]{64}$"))) | .name] as $uploaded
             | ["MusicX-macos-arm64.dmg", "MusicX-macos-arm64.dmg.sha256", "appcast.xml", "release-manifest.json"]
               - $uploaded == []))')

if [[ "$published" == true ]]; then
    echo "Preview $tag already has all published assets."
    exit 0
fi

gh workflow run release-macos.yml --repo "$repo" --ref "$tag"
