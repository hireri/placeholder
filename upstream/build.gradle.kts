plugins {
    id("org.jetbrains.kotlin.jvm")
    id("org.jetbrains.kotlin.plugin.serialization")
}

// Vendored from Metrolist v13.7.0 (GPL-3.0, see UPSTREAM.txt): lyrics providers, Last.fm client, preference keys, InnerTubeX player.
// Android-bound symbols they touch are faked in shims/. Our own adapters sit next to them in src/.

dependencies {
    api(project(":innertube"))
    api(project(":shims"))
    api(libs.json)
    api(libs.datastore)
    api(libs.kuromoji.ipadic)
}
