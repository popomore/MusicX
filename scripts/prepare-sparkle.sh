#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

# Pin both the upstream release and its SHA-256. Never fetch executable code
# from a moving "latest" URL during packaging.
version=2.10.1
digest=1a856d2a9edb8e628de88121d7be04cf0e2167f8641650b78fca00b14244cf27
destination="$PWD/.tools/sparkle-$version"
if [[ ! -f "$destination/.verified-$digest" ]]; then
    stage=$(mktemp -d "${TMPDIR:-/tmp}/musicx-sparkle.XXXXXX")
    trap 'rm -rf "$stage"' EXIT
    curl --fail --location --silent --show-error --proto '=https' --tlsv1.2 \
        "https://github.com/sparkle-project/Sparkle/releases/download/$version/Sparkle-$version.tar.xz" \
        -o "$stage/sparkle.tar.xz"
    printf '%s  %s\n' "$digest" "$stage/sparkle.tar.xz" | shasum -a 256 --check >&2
    mkdir "$stage/distribution"
    tar -xJf "$stage/sparkle.tar.xz" -C "$stage/distribution"
    test -f "$stage/distribution/Sparkle.framework/Sparkle"
    mkdir -p .tools
    # A previous unverified or interrupted cache must not be reused.
    rm -rf "$destination"
    mv "$stage/distribution" "$destination"
    touch "$destination/.verified-$digest"
fi
printf '%s\n' "$destination"
