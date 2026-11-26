plugins {
    id("org.jetbrains.kotlin.jvm")
    id("org.jetbrains.kotlin.plugin.serialization")
}

// BitChord's JVM-shared Spotify library, compiled unmodified from vendor/bitchord (see vendor/bitchord submodule).
sourceSets.main {
    kotlin.srcDir(rootDir.resolve("vendor/bitchord/shared/src/jvmSharedMain/kotlin"))
    kotlin.include(
        "com/music/bitchord/data/Http.kt",
        "com/music/bitchord/data/DebugLog.kt",
        "com/music/bitchord/data/webdav/WebDavAuth.kt",
        "com/music/bitchord/data/spotify/SpotifyLibrary.kt",
        "com/music/bitchord/data/innertube/potoken/PoTokenPage.kt",
        "com/music/bitchord/data/innertube/potoken/JavaScriptUtil.kt",
        "com/music/bitchord/data/innertube/potoken/PoTokenException.kt",
    )
}

dependencies {
    api(project(":innertube")) // brings okhttp + kotlinx-serialization transitively via ktor
    api("com.squareup.okhttp3:okhttp:5.1.0")
}
