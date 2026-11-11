#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
: "${SPARKLE_PRIVATE_KEY:?Configure the Sparkle Ed25519 signing key}"
: "${GITHUB_REPOSITORY:?}"
: "${RELEASE_TAG:=${GITHUB_REF_NAME:?}}"
channel=$(bash scripts/validate-release-tag.sh HEAD "$RELEASE_TAG")
dmg=$1
sparkle=$(bash scripts/prepare-sparkle.sh)
stage=$(mktemp -d "${TMPDIR:-/tmp}/musicx-appcast.XXXXXX")
trap 'rm -rf "$stage"' EXIT
# Isolate this release; other signed items are merged separately.
cp "$dmg" "$stage/"
options=()
if [[ "$channel" == preview ]]; then options+=(--channel preview); fi
printf '%s' "$SPARKLE_PRIVATE_KEY" | "$sparkle/bin/generate_appcast" \
    --ed-key-file - --maximum-deltas 0 \
    "${options[@]}" \
    --download-url-prefix "https://github.com/$GITHUB_REPOSITORY/releases/download/$RELEASE_TAG/" \
    --link "https://github.com/$GITHUB_REPOSITORY/releases/tag/$RELEASE_TAG" "$stage"
test -s "$stage/appcast.xml"
python3 - "$stage/appcast.xml" "${RELEASE_TAG#v}" <<'PY'
from pathlib import Path
import sys
import xml.etree.ElementTree as ET
ET.register_namespace('sparkle', 'http://www.andymatuschak.org/xml-namespaces/sparkle')
path = Path(sys.argv[1])
tree = ET.parse(path)
item = tree.find('./channel/item')
key = '{http://www.andymatuschak.org/xml-namespaces/sparkle}shortVersionString'
element = item.find(key)
if element is None:
    element = ET.SubElement(item, key)
element.text = sys.argv[2]
tree.write(path, encoding='utf-8', xml_declaration=True)
PY
printf '%s' "$SPARKLE_PRIVATE_KEY" | "$sparkle/bin/sign_update" --ed-key-file - "$stage/appcast.xml"
printf '%s' "$SPARKLE_PRIVATE_KEY" | "$sparkle/bin/sign_update" \
    --verify --ed-key-file - "$stage/appcast.xml"
cp "$stage/appcast.xml" "$(dirname "$dmg")/appcast.xml"
