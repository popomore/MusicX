#!/usr/bin/env bash
set -euo pipefail

if [[ "$(git rev-parse HEAD)" != "$COMMIT_SHA" ]]; then
  echo 'The deployment checkout must match the checked commit' >&2
  exit 1
fi

case "$TARGET_ENVIRONMENT" in
  test)
    if [[ "$EVENT:$REF_TYPE:$REF_NAME" != push:branch:main ]]; then
      echo 'Automatic test deployments require a main push' >&2
      exit 1
    fi
    latest="$(git ls-remote --exit-code origin refs/heads/main)"
    read -r latest_sha latest_ref <<< "$latest"
    if [[ ! "$latest_sha" =~ ^[0-9a-f]{40}$ || "$latest_ref" != refs/heads/main ]]; then
      echo 'Cannot verify the current main commit' >&2
      exit 1
    fi
    if [[ "$COMMIT_SHA" != "$latest_sha" ]]; then
      echo 'Skipping an obsolete main build'
      exit 0
    fi
    ;;
  production)
    if [[ "$EVENT:$REF_TYPE" != push:tag ]]; then
      echo 'Production deployments require a validated tag push' >&2
      exit 1
    fi
    ;;
  *) echo 'Unsupported deployment environment' >&2; exit 1 ;;
esac

test -n "$CLOUDFLARE_API_TOKEN" || { echo 'Missing CLOUDFLARE_API_TOKEN' >&2; exit 1; }
test -n "$CLOUDFLARE_ACCOUNT_ID" || { echo 'Missing CLOUDFLARE_ACCOUNT_ID' >&2; exit 1; }
npm exec -- wrangler deploy --env "$TARGET_ENVIRONMENT"
