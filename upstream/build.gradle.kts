import org.jetbrains.kotlin.gradle.tasks.KotlinCompile

plugins {
    id("org.jetbrains.kotlin.jvm")
    id("org.jetbrains.kotlin.plugin.serialization")
}

// Subset of upstream's :app sources compiled unmodified. This list is the whole "port":
// to bring another upstream package over, add it here and fix what the compiler reports
// by extending shims/ (never by editing upstream files).
val app = rootDir.resolve("metrolist/app/src/main/kotlin")
val upstreamSources = fileTree(app) {
    include(
        "com/metrolist/lrclib/**",
        "com/metrolist/kugou/**",
        "com/metrolist/lastfm/**",
        "com/metrolist/paxsenix/**",
        "com/metrolist/music/betterlyrics/**",
        "com/metrolist/music/api/**",
        "com/metrolist/music/constants/**",
        "com/metrolist/music/lyrics/**",
        "com/metrolist/music/utils/InnerTubeXPlayer.kt",
        "com/metrolist/music/utils/potoken/PoTokenResult.kt",
    )
    exclude(
        // UI-only files that live next to backend ones.
        "com/metrolist/music/constants/Dimensions.kt",
        "com/metrolist/music/constants/MediaSessionConstants.kt",
        // DB-bound; the daemon has its own thin LyricsService over the providers.
        "com/metrolist/music/lyrics/LyricsHelper.kt",
        "com/metrolist/music/lyrics/LyricsTranslationHelper.kt",
    )
}

tasks.named<KotlinCompile>("compileKotlin") { source(upstreamSources) }

dependencies {
    api(project(":innertube"))
    api(project(":shims"))
    api(libs.json)
    api(libs.datastore)
    api(libs.kuromoji.ipadic)
}
