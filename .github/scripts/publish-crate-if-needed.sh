#!/usr/bin/env bash

set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "Usage: $0 <crate-name>" >&2
  exit 2
fi

crate_name="$1"
metadata="$(cargo metadata --format-version 1 --no-deps)"
version="$(
  jq --exit-status --raw-output --arg name "$crate_name" \
    '.packages[] | select(.name == $name) | .version' <<< "$metadata"
)"

response_file="$(mktemp)"
trap 'rm -f "$response_file"' EXIT

status="$(
  curl --silent --show-error \
    --output "$response_file" \
    --write-out '%{http_code}' \
    "https://crates.io/api/v1/crates/${crate_name}/${version}"
)"

case "$status" in
  200)
    registry_version="$(jq --exit-status --raw-output '.version.num' "$response_file")"
    if [[ "$registry_version" != "$version" ]]; then
      echo "crates.io returned ${registry_version} for ${crate_name}@${version}" >&2
      exit 1
    fi

    echo "${crate_name}@${version} is already published, skipping"
    exit 0
    ;;
  404)
    ;;
  *)
    echo "crates.io lookup for ${crate_name}@${version} failed with HTTP ${status}" >&2
    exit 1
    ;;
esac

cargo publish -p "$crate_name"
