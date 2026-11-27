#!/usr/bin/env bash
set -euo pipefail

if [[ "$EVENT" == pull_request ]]; then
  exit 0
fi

if [[ "$EVENT:$REF_TYPE:$REF_NAME" == push:branch:main ]]; then
  target=test
else
  case "$EVENT:$REF_TYPE" in
    push:tag) tag="$REF_NAME" ;;
    workflow_dispatch:*) tag="$INPUT_TAG" ;;
    *) echo 'Unsupported site release event' >&2; exit 1 ;;
  esac
  if [[ ! "$tag" =~ ^site-v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
    echo 'Expected an existing stable site-vX.Y.Z tag' >&2
    exit 1
  fi
  if [[ "$(git rev-parse --verify "refs/tags/$tag^{commit}")" != "$(git rev-parse HEAD)" ]]; then
    echo 'The checkout does not match the site tag' >&2
    exit 1
  fi
  if ! git merge-base --is-ancestor HEAD refs/remotes/origin/main; then
    echo 'The site tag must point to a commit reachable from main' >&2
    exit 1
  fi
  target=production
fi

echo "target_environment=$target" >> "$GITHUB_OUTPUT"
