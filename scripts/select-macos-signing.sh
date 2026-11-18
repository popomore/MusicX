#!/usr/bin/env bash
set -euo pipefail

required=(APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY
          APPLE_API_PRIVATE_KEY APPLE_API_KEY APPLE_API_ISSUER)
missing=()
for name in "${required[@]}"; do
    if [[ -z "${!name:-}" ]]; then
        missing+=("$name")
    fi
done

if (( ${#missing[@]} )); then
    printf '::error::Stable releases require Developer ID signing and notarization. Missing: %s\n' "${missing[*]}" >&2
    exit 1
fi
if [[ "$APPLE_SIGNING_IDENTITY" != "Developer ID Application: "?* ]]; then
    echo '::error::APPLE_SIGNING_IDENTITY must be a Developer ID Application identity.' >&2
    exit 1
fi
printf 'developer-id\n'
