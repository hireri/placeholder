// Desktop backend build. Separate Gradle build so no upstream file is ever edited:
// run it with `../gradlew -p desktop <task>`.
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
    versionCatalogs {
        // Same catalog as upstream, so version bumps (innertubex, ktor, ...) arrive with every merge.
        create("libs") { from(files("../gradle/libs.versions.toml")) }
    }
}

include(":shims", ":innertube", ":upstream", ":bitchord", ":daemon")
