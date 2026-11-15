#!/usr/bin/env bash
# Check out only what the build reads from the submodules (see upstream/build.gradle.kts, bitchord/build.gradle.kts).
# Pins and Dependabot are unaffected; `git sparse-checkout disable` in a submodule restores everything.
set -euo pipefail
cd "$(dirname "$0")"
git -C metrolist sparse-checkout set gradle innertube app/src/main/kotlin/com/metrolist
git -C vendor/bitchord sparse-checkout set shared/src/jvmSharedMain/kotlin
