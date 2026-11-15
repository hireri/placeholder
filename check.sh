#!/usr/bin/env bash
# Drift detector: run after merging upstream or bumping vendor.lock. A compile error here means
# upstream changed something the shims or adapters depend on; fix it in shims/, not upstream files.
set -euo pipefail
cd "$(dirname "$0")"
metrolist/gradlew -p . test :daemon:installDist
(cd client && cargo check --workspace)
