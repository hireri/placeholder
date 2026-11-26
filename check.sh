#!/usr/bin/env bash
# Build check: run before pushing and after bumping a dependency or the bitchord submodule.
set -euo pipefail
cd "$(dirname "$0")"
./gradlew test :daemon:installDist
(cd client && cargo check --workspace)
