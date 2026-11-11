#!/usr/bin/env bash
set -euo pipefail

required=(APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY
          APPLE_API_PRIVATE_KEY APPLE_API_KEY APPLE_API_ISSUER)
missing=()
configured=0
for name in "${required[@]}"; do
    if [[ -n "${!name:-}" ]]; then
        configured=$((configured + 1))
    else
        missing+=("$name")
    fi
done

if (( configured == 0 )); then
    echo 'Apple signing is not configured; using ad hoc signing.' >&2
    printf 'ad-hoc\n'
elif (( configured == ${#required[@]} )); then
    printf 'developer-id\n'
else
    printf '::error::Incomplete Apple signing configuration. Missing: %s\n' "${missing[*]}" >&2
    exit 1
fi
