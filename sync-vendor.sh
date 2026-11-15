#!/usr/bin/env bash
# Fetch the pinned commit of every repo in vendor.lock into vendor/<name> (gitignored).
set -euo pipefail
cd "$(dirname "$0")"
grep -vE '^\s*(#|$)' vendor.lock | while read -r name url sha; do
  dir="vendor/$name"
  if [ -d "$dir/.git" ] && [ "$(git -C "$dir" rev-parse HEAD 2>/dev/null)" = "$sha" ]; then
    echo "$name: up to date ($sha)"; continue
  fi
  rm -rf "$dir"; mkdir -p "$dir"
  git -C "$dir" init -q
  git -C "$dir" fetch -q --depth 1 "$url" "$sha"
  git -C "$dir" checkout -q FETCH_HEAD
  echo "$name: fetched $sha"
done
