plugins {
    id("org.jetbrains.kotlin.jvm")
    id("org.jetbrains.kotlin.plugin.serialization")
}

// Vendored from Metrolist v13.7.0's :innertube module (GPL-3.0, see UPSTREAM.txt). It has no Android dependencies.

dependencies {
    api(project(":shims"))
    api(libs.innertubex)
    api(libs.ktor.client.core)
    api(libs.ktor.client.okhttp)
    api(libs.ktor.client.content.negotiation)
    api(libs.ktor.client.encoding)
    api(libs.ktor.serialization.json)
}
