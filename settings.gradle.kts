// Desktop backend build: `./gradlew <task>`.
rootProject.name = "metrolist-desktop"

pluginManagement {
    repositories {
        mavenLocal()
        gradlePluginPortal()
        mavenCentral()
        google()
    }
}

dependencyResolutionManagement {
    repositories {
        exclusiveContent {
            forRepository { maven("https://jitpack.io") }
            filter { includeGroup("com.github.MetrolistGroup.innertubex") }
        }
        mavenCentral()
        google()
        maven("https://jitpack.io")
    }
    // gradle/libs.versions.toml is picked up as `libs` automatically. Copied from Metrolist v13.7.0; versions are ours to bump now.
}

include(":shims", ":innertube", ":upstream", ":bitchord", ":daemon")
