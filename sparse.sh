#!/usr/bin/env bash
# Check out only what the build reads from the BitChord submodule (see bitchord/build.gradle.kts).
# `git sparse-checkout disable` in the submodule restores everything.
set -euo pipefail
cd "$(dirname "$0")"
git -C vendor/bitchord sparse-checkout set shared/src/jvmSharedMain/kotlin
